use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Master on/off switch for the layout-switch hotkey.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub hotkey: Hotkey,
    /// Whether the app is registered in HKCU\...\Run.
    #[serde(default)]
    pub autostart: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: Hotkey::default(),
            autostart: false,
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    use super::Config;

    pub fn config_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("rasklad").join("config.json")
    }

    pub fn load() -> Config {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(cfg: &Config) -> io::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_string_pretty(cfg).unwrap_or_default())?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Register/unregister the app in HKCU\...\Run.
    pub fn set_autostart(enable: bool) -> io::Result<()> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) =
            hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
        if enable {
            let exe = std::env::current_exe()?;
            let value = format!("\"{}\" --minimized", exe.display());
            key.set_value("rasklad", &value)?;
        } else {
            let _ = key.delete_value("rasklad");
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_defaults() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.hotkey, Hotkey::default());
        assert!(!cfg.autostart);
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = Config {
            enabled: false,
            autostart: true,
            ..Config::default()
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert!(!back.enabled);
        assert!(back.autostart);
    }
}
