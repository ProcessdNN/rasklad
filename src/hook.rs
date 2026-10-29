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
    /// Key whose press we swallowed. Its release has to be swallowed as well,
    /// or the system is left with a key that went down and never came up -
    /// Caps Lock then stays latched and stops responding.
    static SWALLOWED: Cell<u32> = const { Cell::new(0) };
}

/// Optional debug log: set `RASKLAD_LOG=<file>` to trace hook decisions.
/// Costs nothing when the variable is unset.
/// What the hook does with one key event.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Action {
    /// Nothing special: Windows and the focused app see the key.
    Pass,
    /// Swallow it.
    Swallow,
    /// Swallow it and switch the layout.
    Switch,
    /// Swallow it and synthesise a bare Caps press.
    ToggleCaps,
}

/// The configuration a single key event depends on.
struct Decision {
    enabled: bool,
    hotkey: Hotkey,
    caps_combo: CapsCombo,
    /// Key whose press this hook swallowed and whose release is still due.
    swallowed: u32,
}

/// Decide what happens to one key event.
///
/// Kept away from the Win32 calls on purpose: this is where the rules live,
/// and they are worth testing without a keyboard and a message loop.
fn decide(vk: u32, mods: Modifiers, down: bool, repeat: bool, d: &Decision) -> Action {
    if !down {
        // Only the release of a press we ate is ours to eat. Testing the
        // hotkey here instead used to swallow Caps releases even while the
        // app was switched off, which left Caps latched down.
        return if d.swallowed == vk {
            Action::Swallow
        } else {
            Action::Pass
        };
    }

    let is_hotkey = d.enabled && d.hotkey.matches(vk, mods);
    // While Caps is the layout trigger, every other Caps press has to be
    // swallowed: Windows toggles caps mode natively on any modifier+Caps.
    let caps_owned = d.enabled && vk == vk::VK_CAPITAL && d.hotkey.vk == vk::VK_CAPITAL;

    if is_hotkey {
        return if repeat {
            // Auto-repeat: the key is already down, don't fire again.
            Action::Swallow
        } else {
            Action::Switch
        };
    }
    if caps_owned && d.caps_combo != CapsCombo::Off && d.caps_combo.matches(&mods) {
        return Action::ToggleCaps;
    }
    if caps_owned {
        return Action::Swallow;
    }
    Action::Pass
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
        let self_injected =
            (kb.flags & LLKHF_INJECTED).0 != 0 && kb.dwExtraInfo == switcher::INJECTED_TAG;
        if !self_injected {
            if let Some(shared) = SHARED.get() {
                let vk_code = kb.vkCode;
                let down = wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;

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
                    } else if CAP_KEY.with(|k| k.get()) == vk_code && CAP_KEY.with(|k| k.get()) != 0
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
                let decision = Decision {
                    enabled,
                    hotkey,
                    caps_combo,
                    swallowed: SWALLOWED.with(|s| s.get()),
                };
                let action = decide(
                    vk_code,
                    mods_for(vk_code),
                    down,
                    is_down(vk_code),
                    &decision,
                );
                match action {
                    Action::Pass => {}
                    Action::Switch => {
                        crate::diag::log("hotkey fired");
                        switcher::switch_layout();
                        SWALLOWED.with(|s| s.set(vk_code));
                        return LRESULT(1);
                    }
                    Action::ToggleCaps => {
                        switcher::tap_key(vk::VK_CAPITAL);
                        SWALLOWED.with(|s| s.set(vk_code));
                        return LRESULT(1);
                    }
                    Action::Swallow => {
                        // A press we ate is remembered so its release is eaten
                        // too; that release then clears the memory.
                        SWALLOWED.with(|s| s.set(if down { vk_code } else { 0 }));
                        return LRESULT(1);
                    }
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
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            Some(HINSTANCE(hmod.0)),
            0,
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    const NO_MODS: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
    };
    const WIN: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
        win: true,
    };
    const CTRL: Modifiers = Modifiers {
        ctrl: true,
        alt: false,
        shift: false,
        win: false,
    };

    fn caps_hotkey() -> Hotkey {
        Hotkey {
            vk: vk::VK_CAPITAL,
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        }
    }

    fn ctrl_space() -> Hotkey {
        Hotkey {
            vk: 0x20,
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
        }
    }

    fn config(enabled: bool, hotkey: Hotkey, caps_combo: CapsCombo) -> Decision {
        Decision {
            enabled,
            hotkey,
            caps_combo,
            swallowed: 0,
        }
    }

    /// Plays a sequence of presses through the same rules the hook uses,
    /// carrying the "press we ate" state the way the callback does.
    struct Sim(Decision);

    impl Sim {
        fn new(enabled: bool, hotkey: Hotkey, caps_combo: CapsCombo) -> Self {
            Self(config(enabled, hotkey, caps_combo))
        }

        fn press(&mut self, vk_code: u32, mods: Modifiers) -> Action {
            let action = decide(vk_code, mods, true, false, &self.0);
            if action != Action::Pass {
                self.0.swallowed = vk_code;
            }
            action
        }

        fn release(&mut self, vk_code: u32) -> Action {
            let action = decide(vk_code, NO_MODS, false, false, &self.0);
            if action == Action::Swallow {
                self.0.swallowed = 0;
            }
            action
        }
    }

    #[test]
    fn switched_off_lets_caps_through_both_ways() {
        // The bug this guards: with the hotkey switched off, presses went to
        // Windows but releases did not, and Caps stayed latched down.
        let mut sim = Sim::new(false, caps_hotkey(), CapsCombo::Win);
        assert_eq!(sim.press(vk::VK_CAPITAL, NO_MODS), Action::Pass);
        assert_eq!(sim.release(vk::VK_CAPITAL), Action::Pass);
        assert_eq!(sim.press(vk::VK_CAPITAL, NO_MODS), Action::Pass);
        assert_eq!(sim.release(vk::VK_CAPITAL), Action::Pass);
    }

    #[test]
    fn switched_on_caps_is_the_hotkey() {
        let mut sim = Sim::new(true, caps_hotkey(), CapsCombo::Off);
        assert_eq!(sim.press(vk::VK_CAPITAL, NO_MODS), Action::Switch);
        assert_eq!(sim.release(vk::VK_CAPITAL), Action::Swallow);
        // And it still works on the next press.
        assert_eq!(sim.press(vk::VK_CAPITAL, NO_MODS), Action::Switch);
        assert_eq!(sim.release(vk::VK_CAPITAL), Action::Swallow);
    }

    #[test]
    fn auto_repeat_does_not_switch_twice() {
        let d = config(true, caps_hotkey(), CapsCombo::Off);
        assert_eq!(
            decide(vk::VK_CAPITAL, NO_MODS, true, true, &d),
            Action::Swallow
        );
        assert_eq!(
            decide(vk::VK_CAPITAL, NO_MODS, true, false, &d),
            Action::Switch
        );
    }

    #[test]
    fn a_press_we_ate_keeps_its_release() {
        // Switched off between the press and the release: the press was
        // eaten, so its release has to be eaten too.
        let d = Decision {
            enabled: false,
            hotkey: caps_hotkey(),
            caps_combo: CapsCombo::Off,
            swallowed: vk::VK_CAPITAL,
        };
        assert_eq!(
            decide(vk::VK_CAPITAL, NO_MODS, false, false, &d),
            Action::Swallow
        );
    }

    #[test]
    fn modifier_caps_toggles_caps_mode() {
        let d = config(true, caps_hotkey(), CapsCombo::Win);
        assert_eq!(
            decide(vk::VK_CAPITAL, WIN, true, false, &d),
            Action::ToggleCaps
        );
    }

    #[test]
    fn caps_combo_off_swallows_without_toggling() {
        let d = config(true, caps_hotkey(), CapsCombo::Off);
        assert_eq!(
            decide(vk::VK_CAPITAL, WIN, true, false, &d),
            Action::Swallow
        );
    }

    #[test]
    fn a_different_hotkey_leaves_caps_alone() {
        let d = config(true, ctrl_space(), CapsCombo::Win);
        assert_eq!(
            decide(vk::VK_CAPITAL, NO_MODS, true, false, &d),
            Action::Pass
        );
        assert_eq!(decide(vk::VK_CAPITAL, WIN, true, false, &d), Action::Pass);
        assert_eq!(
            decide(vk::VK_CAPITAL, NO_MODS, false, false, &d),
            Action::Pass
        );
    }

    #[test]
    fn a_different_hotkey_fires_and_keeps_its_release() {
        let mut sim = Sim::new(true, ctrl_space(), CapsCombo::Win);
        assert_eq!(sim.press(0x20, CTRL), Action::Switch);
        assert_eq!(sim.release(0x20), Action::Swallow);
        // A release we never ate stays untouched.
        assert_eq!(sim.release(0x20), Action::Pass);
    }

    #[test]
    fn nothing_is_swallowed_while_switched_off() {
        // Whatever the key, an off app must not interfere.
        let d = config(false, ctrl_space(), CapsCombo::Shift);
        for vk_code in [vk::VK_CAPITAL, 0x20, 0x41] {
            assert_eq!(decide(vk_code, NO_MODS, true, false, &d), Action::Pass);
            assert_eq!(decide(vk_code, NO_MODS, false, false, &d), Action::Pass);
        }
    }
}
