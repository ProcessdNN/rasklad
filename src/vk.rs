//! Virtual-key codes used across the app.
//! Kept as plain `u32` so this module compiles and tests on any platform.

pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12; // Alt
pub const VK_PAUSE: u32 = 0x13;
pub const VK_CAPITAL: u32 = 0x14;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_PRIOR: u32 = 0x21; // Page Up
pub const VK_NEXT: u32 = 0x22; // Page Down
pub const VK_END: u32 = 0x23;
pub const VK_HOME: u32 = 0x24;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_SNAPSHOT: u32 = 0x2C; // Print Screen
pub const VK_INSERT: u32 = 0x2D;
pub const VK_DELETE: u32 = 0x2E;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;
pub const VK_APPS: u32 = 0x5D;
pub const VK_NUMLOCK: u32 = 0x90;
pub const VK_SCROLL: u32 = 0x91;

/// Keys that act as modifiers and are therefore never treated as a
/// hotkey's trigger key (they only contribute to the modifier mask).
pub fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// Human-readable key name shown in the settings window.
pub fn name(vk: u32) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        VK_BACK => "Backspace".into(),
        VK_TAB => "Tab".into(),
        VK_RETURN => "Enter".into(),
        VK_SHIFT => "Shift".into(),
        VK_CONTROL => "Ctrl".into(),
        VK_MENU => "Alt".into(),
        VK_PAUSE => "Pause".into(),
        VK_CAPITAL => "Caps Lock".into(),
        VK_ESCAPE => "Esc".into(),
        VK_SPACE => "Space".into(),
        VK_PRIOR => "PageUp".into(),
        VK_NEXT => "PageDown".into(),
        VK_END => "End".into(),
        VK_HOME => "Home".into(),
        VK_LEFT => "Left".into(),
        VK_UP => "Up".into(),
        VK_RIGHT => "Right".into(),
        VK_DOWN => "Down".into(),
        VK_SNAPSHOT => "PrintScreen".into(),
        VK_INSERT => "Insert".into(),
        VK_DELETE => "Delete".into(),
        VK_LWIN => "LWin".into(),
        VK_RWIN => "RWin".into(),
        VK_APPS => "Menu".into(),
        VK_NUMLOCK => "NumLock".into(),
        VK_SCROLL => "ScrollLock".into(),
        0x6A => "Num*".into(),
        0x6B => "Num+".into(),
        0x6D => "Num-".into(),
        0x6E => "Num.".into(),
        0x6F => "Num/".into(),
        0xBA => ";".into(),
        0xBB => "=".into(),
        0xBC => ",".into(),
        0xBD => "-".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0xC0 => "`".into(),
        0xDB => "[".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        0xDE => "'".into(),
        _ => format!("Key 0x{vk:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_detected() {
        for vk in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA5] {
            assert!(is_modifier(vk), "{vk:#x}");
        }
        for vk in [0x14, 0x20, 0x41, 0x5D, 0xA6] {
            assert!(!is_modifier(vk), "{vk:#x}");
        }
    }

    #[test]
    fn names() {
        assert_eq!(name(VK_CAPITAL), "Caps Lock");
        assert_eq!(name(0x41), "A");
        assert_eq!(name(0x70), "F1");
        assert_eq!(name(0x7B), "F12");
        assert_eq!(name(0x99), "Key 0x99");
    }
}
