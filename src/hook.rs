//! Low-level keyboard hook (WH_KEYBOARD_LL) running on the worker thread's
//! message loop. It watches for the configured hotkey, suppresses it and
//! triggers a layout switch. Only keystrokes we injected ourselves
//! (tagged in dwExtraInfo by switcher) are ignored — injected input from
//! other sources (VMs, RDP, AutoHotkey) is treated as real keys, so the
//! hotkey also works on remote/virtual machines.
//!
//! Modifier state is read from `GetAsyncKeyState` instead of being
//! accumulated from key events: a single missed key-up (focus switches,
//! Win+Tab, RDP, a hook that Windows temporarily drops) used to leave a
//! modifier stuck forever, which silently broke the hotkey until restart.
//! Same for the key-repeat guard: it asks the OS whether the key was
//! already down, so there is no state to get stuck either.

#![cfg(windows)]

use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::CapsCombo;
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
    /// Capture-mode candidate key, finalized when it is released.
    static CAP_KEY: Cell<u32> = const { Cell::new(0) };
    static CAP_MODS: Cell<u8> = const { Cell::new(0) };
}

/// Optional debug log: set `RASKLAD_LOG=<file>` to trace hook decisions.
/// Costs nothing when the variable is unset.
fn mod_bit(vk_code: u32) -> u8 {
    match vk_code {
        0x10 | 0xA0 | 0xA1 => M_SHIFT, // Shift / LShift / RShift
        0x11 | 0xA2 | 0xA3 => M_CTRL,  // Control / LControl / RControl
        0x12 | 0xA4 | 0xA5 => M_ALT,   // Menu / LMenu / RMenu
        0x5B | 0x5C => M_WIN,          // LWin / RWin
        _ => 0,
    }
}

/// Modifiers physically held right now, straight from the OS.
fn held_mods() -> u8 {
    unsafe {
        let mut m = 0u8;
        if GetAsyncKeyState(vk::VK_LWIN as i32) < 0 || GetAsyncKeyState(vk::VK_RWIN as i32) < 0 {
            m |= M_WIN;
        }
        if GetAsyncKeyState(vk::VK_SHIFT as i32) < 0 {
            m |= M_SHIFT;
        }
        if GetAsyncKeyState(vk::VK_CONTROL as i32) < 0 {
            m |= M_CTRL;
        }
        if GetAsyncKeyState(vk::VK_MENU as i32) < 0 {
            m |= M_ALT;
        }
        m
    }
}

/// Modifiers physically held right now — used by the in-window hotkey
/// capture, which cannot rely on the hook (see ui::App::poll_capture).
pub fn current_modifiers() -> Modifiers {
    modifiers_of(held_mods())
}

/// Is this key physically down right now?
pub fn key_down(vk_code: u32) -> bool {
    is_down(vk_code)
}

/// Is `vk` physically down right now? Used to tell a fresh press from
/// keyboard auto-repeat (a repeat arrives while the key is already down).
fn is_down(vk_code: u32) -> bool {
    unsafe { GetAsyncKeyState(vk_code as i32) < 0 }
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
/// modifier bit is stripped so a modifier used AS the trigger still matches.
fn mods_for(vk_code: u32) -> Modifiers {
    modifiers_of(held_mods() & !mod_bit(vk_code))
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let self_injected = (kb.flags & LLKHF_INJECTED).0 != 0
            && kb.dwExtraInfo == switcher::INJECTED_TAG;
        if !self_injected {
            if let Some(shared) = SHARED.get() {
                let vk_code = kb.vkCode;
                let down =
                    wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;

                if shared.capturing.load(Ordering::SeqCst) {
                    // Capture mode: the last real key pressed becomes the
                    // hotkey, with the modifiers held when it went down.
                    // Bare modifiers never become the hotkey, and a combo
                    // that would be useless (a plain letter, or a modifier
                    // on its own) is rejected with a hint while capture
                    // keeps running. Everything is suppressed.
                    if down {
                        if vk_code == vk::VK_ESCAPE {
                            CAP_KEY.with(|k| k.set(0));
                            shared.capturing.store(false, Ordering::SeqCst);
                            *shared.capture.lock().unwrap() = Some(CaptureResult::Cancelled);
                        } else if mod_bit(vk_code) == 0 {
                            CAP_KEY.with(|k| k.set(vk_code));
                            CAP_MODS.with(|c| c.set(held_mods() & !mod_bit(vk_code)));
                        }
                    } else if CAP_KEY.with(|k| k.get()) == vk_code
                        && CAP_KEY.with(|k| k.get()) != 0
                    {
                        let m = modifiers_of(CAP_MODS.with(|m| m.get()));
                        CAP_KEY.with(|k| k.set(0));
                        let hotkey = Hotkey {
                            vk: vk_code,
                            ctrl: m.ctrl,
                            alt: m.alt,
                            shift: m.shift,
                            win: m.win,
                        };
                        match hotkey.validate() {
                            Ok(()) => {
                                shared.capturing.store(false, Ordering::SeqCst);
                                *shared.capture.lock().unwrap() =
                                    Some(CaptureResult::Captured(hotkey));
                            }
                            Err(hint) => {
                                // Keep capturing — the user just gets told
                                // what is wrong with this combo.
                                *shared.capture.lock().unwrap() =
                                    Some(CaptureResult::Rejected(hint));
                            }
                        }
                    }
                    return LRESULT(1);
                }

                let (hotkey, caps_combo, enabled) = {
                    let cfg = shared.config.lock().unwrap();
                    (cfg.hotkey, cfg.caps_combo, cfg.enabled)
                };
                let is_hotkey = enabled && hotkey.matches(vk_code, mods_for(vk_code));
                // While Caps is the layout trigger, every other Caps press
                // must be swallowed — otherwise Windows toggles caps mode
                // natively on ANY modifier+Caps. Only the configured combo
                // gets through, as a synthetic tagged bare-Caps press.
                // When the app is switched off we must not touch Caps at
                // all: eating the key while doing nothing makes the app look
                // broken instead of disabled.
                let caps_owned =
                    enabled && vk_code == vk::VK_CAPITAL && hotkey.vk == vk::VK_CAPITAL;

                if down {
                    let inject_caps = !is_hotkey
                        && caps_owned
                        && caps_combo != CapsCombo::Off
                        && caps_combo.matches(&mods_for(vk_code));
                    if is_hotkey {
                        // Auto-repeat: the key is already down, don't fire again.
                        if !is_down(vk_code) {
                            crate::diag::log("hotkey fired");
                            switcher::switch_layout();
                        }
                        return LRESULT(1);
                    }
                    if inject_caps {
                        switcher::tap_key(vk::VK_CAPITAL);
                    }
                    if caps_owned {
                        return LRESULT(1);
                    }
                } else if vk_code == hotkey.vk {
                    // Swallow the matching key-up too, so the app never sees
                    // a dangling release for a key we ate.
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
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(HINSTANCE(hmod.0)), 0);
        match &hook {
            Ok(h) => crate::diag::log(&format!("hook installed {:?}", h.0)),
            Err(e) => crate::diag::log(&format!("hook install FAILED: {e:?}")),
        }
        hook
    }
}

pub fn uninstall(hhk: HHOOK) {
    unsafe {
        let _ = UnhookWindowsHookEx(hhk);
    }
}
