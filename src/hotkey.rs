use serde::{Deserialize, Serialize};

use crate::vk;

/// Trigger for the layout switch: a key plus the exact modifier set that
/// must be held. Extra modifiers do NOT trigger it — the match is exact,
/// so `Caps Lock` only fires on a bare Caps press and `Shift+Caps` keeps
/// its normal meaning (toggling caps mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub vk: u32,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub win: bool,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            vk: vk::VK_CAPITAL,
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        }
    }
}

/// Currently pressed modifiers, tracked by the keyboard hook.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Hotkey {
    /// True when `vk` went down while exactly this hotkey's modifiers are held.
    pub fn matches(&self, vk: u32, mods: Modifiers) -> bool {
        self.vk == vk
            && self.ctrl == mods.ctrl
            && self.alt == mods.alt
            && self.shift == mods.shift
            && self.win == mods.win
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        parts.push(vk::name(self.vk));
        parts.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: u32 = 0x14;

    #[test]
    fn default_is_caps_lock() {
        let hk = Hotkey::default();
        assert_eq!(hk.vk, CAPS);
        assert!(hk.matches(CAPS, Modifiers::default()));
        assert_eq!(hk.display(), "Caps Lock");
    }

    #[test]
    fn bare_key_needs_no_modifiers() {
        let hk = Hotkey::default();
        let mods = Modifiers {
            shift: true,
            ..Default::default()
        };
        // Shift+Caps must NOT switch layout — it toggles caps mode instead.
        assert!(!hk.matches(CAPS, mods));
    }

    #[test]
    fn combo_needs_exact_modifiers() {
        let hk = Hotkey {
            vk: 0x20, // Space
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
        };
        assert!(hk.matches(
            0x20,
            Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
        // Missing Ctrl.
        assert!(!hk.matches(0x20, Modifiers::default()));
        // Extra Alt.
        assert!(!hk.matches(
            0x20,
            Modifiers {
                ctrl: true,
                alt: true,
                ..Default::default()
            }
        ));
        // Wrong key.
        assert!(!hk.matches(
            0x0D,
            Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
    }

    #[test]
    fn display_combo() {
        let hk = Hotkey {
            vk: 0x20,
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(hk.display(), "Ctrl+Shift+Space");
    }
}
