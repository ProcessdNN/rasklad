//! Performs the actual layout switch by injecting the system's own
//! layout-switch keystroke via SendInput. Injected input is handled by the
//! OS rather than delivered as a window message, so it is not filtered by
//! UIPI and works even when the foreground window is elevated.

#![cfg(windows)]

use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// dwExtraInfo tag stamped on every keystroke we inject, so the keyboard
/// hook can tell our own synthetic input from injected input coming from
/// elsewhere (VMs, RDP, AutoHotkey) — which we deliberately do NOT ignore.
pub const INJECTED_TAG: usize = 0x524B4C44; // "RKLD"

#[derive(Clone, Copy)]
enum Combo {
    AltShift,
    CtrlShift,
    WinSpace,
}

/// Reads which key sequence Windows uses to cycle input languages
/// (HKCU\Keyboard Layout\Toggle). Falls back to Win+Space, which always
/// works on Windows 10/11 and cannot be unbound.
fn configured_combo() -> Combo {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let value = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Keyboard Layout\\Toggle")
        .and_then(|k| {
            k.get_value::<String, _>("Hotkey")
                .or_else(|_| k.get_value::<String, _>("Language Hotkey"))
        })
        .unwrap_or_else(|_| "1".to_string());

    match value.as_str() {
        "2" => Combo::CtrlShift,
        "3" | "4" => Combo::WinSpace, // disabled / grave accent -> universal fallback
        _ => Combo::AltShift,
    }
}

/// Switch the keyboard layout of the foreground window to the next one.
pub fn switch_layout() {
    match configured_combo() {
        Combo::AltShift => send_combo(&[VK_LMENU.0], VK_LSHIFT.0),
        Combo::CtrlShift => send_combo(&[VK_LCONTROL.0], VK_LSHIFT.0),
        Combo::WinSpace => send_combo(&[VK_LWIN.0], VK_SPACE.0),
    }
}

/// Inject a single key tap (down+up), tagged as ours.
pub fn tap_key(vk: u32) {
    send(&[(vk as u16, true), (vk as u16, false)]);
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
