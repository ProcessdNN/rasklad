//! Layout switching for the foreground window.
//!
//! The first implementation injected the system's own switch hotkey
//! (Alt+Shift) with `SendInput`. That silently does nothing when that hotkey
//! is unbound — which is common on Windows 10/11, where only Win+Space is
//! guaranteed to work. So instead we ask the foreground window to activate
//! the next layout directly (`WM_INPUTLANGCHANGEREQUEST`): no synthetic keys,
//! no language-switcher overlay, and it works for our own window too.
//!
//! `SendInput` is only the fallback, for the one case the message cannot
//! cover: an elevated foreground window rejects it (UIPI).

#![cfg(windows)]

use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
};

/// dwExtraInfo tag stamped on every keystroke we inject, so the keyboard
/// hook can tell our own synthetic input from injected input coming from
/// elsewhere (VMs, RDP, AutoHotkey) — which we deliberately do NOT ignore.
pub const INJECTED_TAG: usize = 0x524B4C44; // "RKLD"

/// Switch the keyboard layout of the foreground window to the next one.
///
/// Runs on a helper thread: the caller is the low-level keyboard hook, which
/// has to return within a few milliseconds or Windows silently unhooks it.
pub fn switch_layout() {
    std::thread::spawn(|| {
        if !switch_via_message() {
            switch_via_hotkey();
        }
    });
}

/// Ask the foreground window to activate `next_layout`.
fn switch_via_message() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return false;
        }
        let tid = GetWindowThreadProcessId(fg, None);
        if tid == 0 {
            return false;
        }
        let Some(next) = next_layout(GetKeyboardLayout(tid)) else {
            return false;
        };
        crate::diag::log(&format!("switch: fg={:?} next={:?}", fg.0, next.0));
        if PostMessageW(
            Some(fg),
            WM_INPUTLANGCHANGEREQUEST,
            WPARAM(0),
            LPARAM(next.0 as isize),
        )
        .is_err()
        {
            return false;
        }
        // The target thread applies it when it next pumps messages; a busy
        // UI thread can take a while, so allow up to ~200 ms before giving
        // up. Waiting here is safe (helper thread). Applying it late is
        // harmless: the message carries the exact layout, so it cannot
        // double-toggle.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            if GetKeyboardLayout(tid) == next {
                crate::diag::log("switch: ok via message");
                return true;
            }
        }
        crate::diag::log("switch: message ignored, falling back to Win+Space");
        false
    }
}

/// Layout after `current` in the installed list, wrapping around.
fn next_layout(current: HKL) -> Option<HKL> {
    unsafe {
        let count = GetKeyboardLayoutList(None);
        if count < 2 {
            return None;
        }
        let mut list = vec![HKL::default(); count as usize];
        let n = GetKeyboardLayoutList(Some(&mut list));
        if n <= 0 {
            return None;
        }
        list.truncate(n as usize);
        let pos = list.iter().position(|h| *h == current)?;
        Some(list[(pos + 1) % list.len()])
    }
}

/// Fallback: inject Win+Space — the one switch shortcut that always works and
/// cannot be unbound. It does flash the language overlay, so it is only used
/// when the message above could not be delivered.
fn switch_via_hotkey() {
    send_combo(&[VK_LWIN.0], VK_SPACE.0);
}

/// Inject a single key tap (down+up), tagged as ours.
///
/// Injected from a helper thread, never from inside the keyboard hook: a
/// hook callback that calls `SendInput` re-enters the input pipeline and
/// Windows can silently drop the hook (or stall it past
/// `LowLevelHooksTimeout`), after which the hotkey stops working until the
/// app is restarted.
pub fn tap_key(vk: u32) {
    std::thread::spawn(move || send(&[(vk as u16, true), (vk as u16, false)]));
}

/// Press mods, tap `key`, release mods.
fn send_combo(mods: &[u16], key: u16) {
    let mut seq = Vec::with_capacity(2 + mods.len() * 2);
    for &m in mods {
        seq.push((m, true));
    }
    seq.push((key, true));
    seq.push((key, false));
    for &m in mods.iter().rev() {
        seq.push((m, false));
    }
    send(&seq);
}

fn send(seq: &[(u16, bool)]) {
    let inputs: Vec<INPUT> = seq
        .iter()
        .map(|&(vk, down)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if down {
                        KEYBD_EVENT_FLAGS(0)
                    } else {
                        KEYEVENTF_KEYUP
                    },
                    time: 0,
                    dwExtraInfo: INJECTED_TAG,
                },
            },
        })
        .collect();
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}
