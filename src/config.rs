use serde::{Deserialize, Deserializer, Serialize};

use crate::hotkey::Hotkey;

/// How rasklad registers itself to start at Windows logon. At most one
/// mechanism is active at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum AutostartMethod {
    #[default]
    None,
    /// HKCU\Software\Microsoft\Windows\CurrentVersion\Run
    RegistryRun,
    /// .lnk in shell:startup
    StartupFolder,
    /// schtasks logon task
    ScheduledTask,
}

impl AutostartMethod {
    pub const ALL: [Self; 3] = [
        Self::RegistryRun,
        Self::StartupFolder,
        Self::ScheduledTask,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Self::None => "Off",
            Self::RegistryRun => "Registry Run key",
            Self::StartupFolder => "Startup folder shortcut",
            Self::ScheduledTask => "Scheduled task (logon)",
        }
    }
}

impl<'de> Deserialize<'de> for AutostartMethod {
    /// Accepts the legacy bool (`true` = registry Run) and variant names in
    /// any case, with or without underscores.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            B(bool),
            S(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::B(true) => Self::RegistryRun,
            Raw::B(false) => Self::None,
            Raw::S(s) => match s.to_ascii_lowercase().replace('_', "").as_str() {
                "registryrun" | "registry" => Self::RegistryRun,
                "startupfolder" => Self::StartupFolder,
                "scheduledtask" => Self::ScheduledTask,
                _ => Self::None,
            },
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Master on/off switch for the layout-switch hotkey.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub hotkey: Hotkey,
    /// Which autostart mechanism is currently active, if any.
    #[serde(default)]
    pub autostart: AutostartMethod,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: Hotkey::default(),
            autostart: AutostartMethod::None,
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    use super::{AutostartMethod, Config};

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

    /// Disable every autostart mechanism, then enable the requested one.
    /// Errors from disabling are ignored (the entry may simply not exist);
    /// a failure to enable the requested method is reported.
    pub fn apply_autostart(method: AutostartMethod) -> io::Result<()> {
        let _ = registry(false);
        let _ = startup_folder(false);
        let _ = scheduled_task(false);
        match method {
            AutostartMethod::None => Ok(()),
            AutostartMethod::RegistryRun => registry(true),
            AutostartMethod::StartupFolder => startup_folder(true),
            AutostartMethod::ScheduledTask => scheduled_task(true),
        }
    }

    /// HKCU\Software\Microsoft\Windows\CurrentVersion\Run
    fn registry(enable: bool) -> io::Result<()> {
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

    fn startup_lnk_path() -> io::Result<PathBuf> {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA not set"))?;
        Ok(base
            .join("Microsoft\\Windows\\Start Menu\\Programs\\Startup")
            .join("rasklad.lnk"))
    }

    /// shell:startup shortcut to the exe.
    fn startup_folder(enable: bool) -> io::Result<()> {
        let lnk = startup_lnk_path()?;
        if enable {
            create_shortcut(&lnk)
        } else {
            match fs::remove_file(&lnk) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }

    fn create_shortcut(lnk: &std::path::Path) -> io::Result<()> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

        fn into(e: windows::core::Error) -> io::Error {
            io::Error::new(io::ErrorKind::Other, e)
        }

        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(into)?;
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(into)?;
            let exe = std::env::current_exe()?;
            let exe_s = HSTRING::from(exe.to_string_lossy().as_ref());
            link.SetPath(&exe_s).map_err(into)?;
            link.SetArguments(&HSTRING::from("--minimized"))
                .map_err(into)?;
            let dir = exe
                .parent()
                .map(|p| HSTRING::from(p.to_string_lossy().as_ref()))
                .unwrap_or_default();
            link.SetWorkingDirectory(&dir).map_err(into)?;
            let persist: IPersistFile = link.cast().map_err(into)?;
            let target = HSTRING::from(lnk.to_string_lossy().as_ref());
            persist.Save(&target, true).map_err(into)?;
        }
        Ok(())
    }

    /// A logon task via schtasks.exe (no COM / admin needed).
    fn scheduled_task(enable: bool) -> io::Result<()> {
        use std::process::Command;

        let (args, op): (Vec<String>, &str) = if enable {
            let exe = std::env::current_exe()?;
            (
                vec![
                    "/Create".into(),
                    "/TN".into(),
                    "Rasklad".into(),
                    "/SC".into(),
                    "ONLOGON".into(),
                    "/RL".into(),
                    "LIMITED".into(),
                    "/F".into(),
                    "/TR".into(),
                    format!("\"{}\" --minimized", exe.display()),
                ],
                "create",
            )
        } else {
            (
                vec![
                    "/Delete".into(),
                    "/TN".into(),
                    "Rasklad".into(),
                    "/F".into(),
                ],
                "delete",
            )
        };
        let out = Command::new("schtasks").args(&args).output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "schtasks {} failed: {}",
                    op,
                    String::from_utf8_lossy(&out.stderr)
                ),
            ))
        }
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
        assert_eq!(cfg.autostart, AutostartMethod::None);
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = Config {
            enabled: false,
            autostart: AutostartMethod::ScheduledTask,
            ..Config::default()
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert!(!back.enabled);
        assert_eq!(back.autostart, AutostartMethod::ScheduledTask);
    }

    #[test]
    fn autostart_legacy_bool() {
        let cfg: Config = serde_json::from_str("{\"autostart\": true}").unwrap();
        assert_eq!(cfg.autostart, AutostartMethod::RegistryRun);
        let cfg: Config = serde_json::from_str("{\"autostart\": false}").unwrap();
        assert_eq!(cfg.autostart, AutostartMethod::None);
    }

    #[test]
    fn autostart_names() {
        assert_eq!(AutostartMethod::RegistryRun.title(), "Registry Run key");
    }
}
