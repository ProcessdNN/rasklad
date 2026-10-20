//! Low-level keyboard hook (WH_KEYBOARD_LL) running on the worker thread's
//! message loop. It watches for the configured hotkey, suppresses it and
//! triggers a layout switch. Only keystrokes we injected ourselves
//! (tagged in dwExtraInfo by switcher) are ignored — injected input from
//! other sources (VMs, RDP, AutoHotkey) is treated as real keys, so the
//! hotkey also works on remote/virtual machines.

#![cfg(windows)]

use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::hotkey::{Hotkey, Modifiers};
use crate::shared::{CaptureResult, Shared};
use crate::switcher;
use crate::vk;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

const M_CTRL: u8 = 1;
const M_ALT: u8 = 2;
const M_SHIFT: u8 = 4;
const M_WIN: u8 = 8;

thread_local! {
    /// Bitmask of currently held modifiers (real key events only).
    static MODS: Cell<u8> = const { Cell::new(0) };
    /// Trigger key currently held down, so key repeats don't re-fire.
    static TRIGGERED: Cell<u32> = const { Cell::new(0) };
    /// Capture-mode candidate: the most recently pressed key plus the
    /// modifiers held when it went down. The combo is finalized when that
    /// key is released — this also lets modifier-only combos (Ctrl+Shift,
    /// Right Alt, ...) be recorded.
    static CAP_KEY: Cell<u32> = const { Cell::new(0) };
    static CAP_MODS: Cell<u8> = const { Cell::new(0) };
}

fn mod_bit(vk_code: u32) -> u8 {
    match vk_code {
        0x10 | 0xA0 | 0xA1 => M_SHIFT, // Shift / LShift / RShift
        0x11 | 0xA2 | 0xA3 => M_CTRL,  // Control / LControl / RControl
        0x12 | 0xA4 | 0xA5 => M_ALT,   // Menu / LMenu / RMenu
        0x5B | 0x5C => M_WIN,          // LWin / RWin
        _ => 0,
    }
}

fn modifiers_of(v: u8) -> Modifiers {
    Modifiers {
        ctrl: v & M_CTRL != 0,
        alt: v & M_ALT != 0,
        shift: v & M_SHIFT != 0,
        win: v & M_WIN != 0,
    }
}

/// Modifiers as the hotkey sees them for `vk_code`: the trigger key's own
/// modifier bit is stripped so a modifier used AS the trigger (e.g.
/// "Ctrl+Shift" = Shift with Ctrl held) still matches.
fn mods_for(vk_code: u32) -> Modifiers {
    modifiers_of(MODS.with(|m| m.get()) & !mod_bit(vk_code))
}

fn update_mods(vk_code: u32, down: bool) {
    let bit = mod_bit(vk_code);
    if bit == 0 {
        return;
    }
    MODS.with(|m| {
        let v = m.get();
        m.set(if down { v | bit } else { v & !bit });
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let self_injected = (kb.flags & LLKHF_INJECTED).0 != 0
            && kb.dwExtraInfo == switcher::INJECTED_TAG;
        if !self_injected {
            if let Some(shared) = SHARED.get() {
                let vk_code = kb.vkCode;
                let down = wparam.0 == WM_KEYDOWN as usize
                    || wparam.0 == WM_SYSKEYDOWN as usize;
                update_mods(vk_code, down);

                if shared.capturing.load(Ordering::SeqCst) {
                    // Capture mode: remember each key pressed; when the most
                    // recent one is released the combo is final. Esc cancels.
                    // Everything is suppressed while capturing.
                    if down {
                        if vk_code == vk::VK_ESCAPE {
                            CAP_KEY.with(|k| k.set(0));
                            shared.capturing.store(false, Ordering::SeqCst);
                            *shared.capture.lock().unwrap() = Some(CaptureResult::Cancelled);
                        } else {
                            CAP_KEY.with(|k| k.set(vk_code));
                            CAP_MODS.with(|c| c.set(MODS.with(|m| m.get()) & !mod_bit(vk_code)));
                        }
                    } else if vk_code == CAP_KEY.with(|k| k.get()) {
                        let m = modifiers_of(CAP_MODS.with(|m| m.get()));
                        CAP_KEY.with(|k| k.set(0));
                        shared.capturing.store(false, Ordering::SeqCst);
                        *shared.capture.lock().unwrap() = Some(CaptureResult::Captured(Hotkey {
                            vk: vk_code,
                            ctrl: m.ctrl,
                            alt: m.alt,
                            shift: m.shift,
                            win: m.win,
                        }));
                    }
                    return LRESULT(1);
                }
                let triggered = TRIGGERED.with(|t| t.get());
                if down {
                    let fire = {
                        let cfg = shared.config.lock().unwrap();
                        cfg.enabled && cfg.hotkey.matches(vk_code, mods_for(vk_code))
                    };
                    if fire {
                        if triggered != vk_code {
                            TRIGGERED.with(|t| t.set(vk_code));
                            switcher::switch_layout();
                        }
                        return LRESULT(1);
                    }
                } else if vk_code == triggered {
                    TRIGGERED.with(|t| t.set(0));
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Install the low-level keyboard hook. The returned HHOOK must stay alive
/// for the hook to remain installed.
pub fn install(shared: Arc<Shared>) -> windows::core::Result<HHOOK> {
    let _ = SHARED.set(shared);
    unsafe {
        let hmod = GetModuleHandleW(PCWSTR::null())?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(HINSTANCE(hmod.0)), 0)
    }
}

pub fn uninstall(hhk: HHOOK) {
    unsafe {
        let _ = UnhookWindowsHookEx(hhk);
    }
}
