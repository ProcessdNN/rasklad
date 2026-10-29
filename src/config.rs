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
    /// Only a scheduled task can start the app with full rights and without
    /// a UAC prompt at logon: the Task Scheduler service launches the
    /// process with the user's elevated token. Registry and Startup-folder
    /// entries always run with the normal token.
    pub fn requires_admin(self) -> bool {
        matches!(self, Self::ScheduledTask)
    }

    /// Stable identifier, used on the helper's command line.
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RegistryRun => "registry",
            Self::StartupFolder => "startup",
            Self::ScheduledTask => "task",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "none" => Some(Self::None),
            "registry" => Some(Self::RegistryRun),
            "startup" => Some(Self::StartupFolder),
            "task" => Some(Self::ScheduledTask),
            _ => None,
        }
    }

    pub const ALL: [Self; 3] = [Self::RegistryRun, Self::StartupFolder, Self::ScheduledTask];

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

/// Which modifier+Caps combo toggles Caps Lock mode. The bare Caps key is
/// normally the layout-switch hotkey, so caps mode itself needs a combo —
/// Shift+Caps Lock is the Windows default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapsCombo {
    Off,
    #[default]
    Shift,
    Ctrl,
    Alt,
    Win,
}

impl CapsCombo {
    pub const ALL: [Self; 5] = [Self::Shift, Self::Ctrl, Self::Alt, Self::Win, Self::Off];

    pub fn title(&self) -> &'static str {
        match self {
            Self::Off => "Off (Caps never toggles)",
            Self::Shift => "Shift+Caps Lock",
            Self::Ctrl => "Ctrl+Caps Lock",
            Self::Alt => "Alt+Caps Lock",
            Self::Win => "Win+Caps Lock",
        }
    }

    /// Does this combo match the given held modifiers?
    /// (`Off` never matches — it means "no combo toggles caps".)
    pub fn matches(&self, m: &crate::hotkey::Modifiers) -> bool {
        let (ctrl, alt, shift, win) = match self {
            Self::Off => return false,
            Self::Shift => (false, false, true, false),
            Self::Ctrl => (true, false, false, false),
            Self::Alt => (false, true, false, false),
            Self::Win => (false, false, false, true),
        };
        m.ctrl == ctrl && m.alt == alt && m.shift == shift && m.win == win
    }
}

/// What the OS actually has registered for autostart.
///
/// The registry value / shortcut / scheduled task outlives the binary it
/// points at: after moving or replacing the exe it can point at an old
/// build, or be an empty command left by an earlier version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutostartState {
    /// Autostart is off in the settings.
    Disabled,
    /// Registered and pointing at the running executable.
    Ok,
    /// Enabled in the settings, but nothing is registered.
    Missing,
    /// Registered, but pointing at some other file (old build, moved exe).
    Stale(String),
    /// Registered, but the command has no usable path.
    Empty,
    /// The check itself failed, so nothing is known about the entry. Kept
    /// apart from `Missing` on purpose: a failed query must not look like a
    /// missing entry and trigger a repair.
    Unknown,
}

impl AutostartState {
    /// True when the entry cannot work at all (nothing registered, an empty
    /// command, or a path that no longer exists). A path that still resolves
    /// to a real file is reported but not rewritten automatically: running a
    /// test build from another folder must not hijack the user's autostart.
    pub fn is_broken(&self) -> bool {
        match self {
            Self::Missing | Self::Empty => true,
            Self::Stale(path) => !std::path::Path::new(path).exists(),
            Self::Disabled | Self::Ok | Self::Unknown => false,
        }
    }

    /// Entry exists but points at a different (still valid) executable.
    pub fn is_stale_but_valid(&self) -> bool {
        matches!(self, Self::Stale(path) if std::path::Path::new(path).exists())
    }

    /// One-line explanation for the UI; `None` when there is nothing to say.
    pub fn message(&self) -> Option<String> {
        match self {
            Self::Disabled | Self::Ok => None,
            Self::Unknown => Some("Could not check the autostart entry.".into()),
            Self::Missing => Some("Autostart entry was missing — recreated.".into()),
            Self::Empty => Some("Autostart entry had no path — repaired.".into()),
            Self::Stale(p) if std::path::Path::new(p).exists() => Some(format!(
                "Autostart points to {p} — not this exe. Re-apply it to repoint."
            )),
            Self::Stale(_) => Some("Autostart pointed to a missing exe — repointed here.".into()),
        }
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
    /// Which modifier+Caps combo toggles Caps Lock mode.
    #[serde(default)]
    pub caps_combo: CapsCombo,
    /// Start into the tray when launched by autostart.
    #[serde(default = "default_true")]
    pub start_minimized: bool,
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
            caps_combo: CapsCombo::Shift,
            start_minimized: true,
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    use super::{AutostartMethod, AutostartState, Config};

    /// Name of the logon task in Task Scheduler.
    const TASK_NAME: &str = "Rasklad";

    /// Forget everything: the next start reads the defaults.
    pub fn delete() {
        let _ = std::fs::remove_file(config_path());
    }

    /// True when a logon task exists, whatever state it is in.
    ///
    /// Used to decide whether removing the autostart needs the elevated
    /// helper: deleting a highest-privileges task needs administrator rights.
    pub fn task_present() -> bool {
        !matches!(
            autostart_state(AutostartMethod::ScheduledTask),
            AutostartState::Missing
        )
    }

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

    /// Read back what the OS has registered and compare it with the exe
    /// that is running right now.
    pub fn autostart_state(method: AutostartMethod) -> AutostartState {
        match method {
            AutostartMethod::None => AutostartState::Disabled,
            AutostartMethod::RegistryRun => registry_state(),
            AutostartMethod::StartupFolder => startup_state(),
            AutostartMethod::ScheduledTask => task_state(),
        }
    }

    fn registry_state() -> AutostartState {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        else {
            return AutostartState::Missing;
        };
        match key.get_value::<String, _>("rasklad") {
            Ok(value) => judge(&value),
            Err(_) => AutostartState::Missing,
        }
    }

    fn startup_state() -> AutostartState {
        let Ok(lnk) = startup_lnk_path() else {
            return AutostartState::Missing;
        };
        if !lnk.exists() {
            return AutostartState::Missing;
        }
        match shortcut_target(&lnk) {
            Some(target) if target.trim().is_empty() => AutostartState::Empty,
            Some(target) => judge(&target),
            // Could not read the link back: recreate it rather than guess.
            None => AutostartState::Empty,
        }
    }

    fn task_state() -> AutostartState {
        let out = match schtasks(&[
            "/Query".into(),
            "/TN".into(),
            TASK_NAME.into(),
            "/XML".into(),
        ]) {
            Ok(out) if out.status.success() => out,
            // schtasks exits with 1 when the task does not exist. Anything
            // else - blocked, timed out, access denied - means the query
            // failed, and reading that as "missing" used to send the app off
            // to rebuild an entry that was probably fine.
            Ok(out) if out.status.code() == Some(1) => return AutostartState::Missing,
            _ => return AutostartState::Unknown,
        };
        let xml = String::from_utf8_lossy(&out.stdout);
        match xml_tag(&xml, "Command") {
            Some(cmd) => judge(&cmd),
            None => AutostartState::Empty,
        }
    }

    /// Turn a command line into a verdict about the running exe.
    fn judge(command: &str) -> AutostartState {
        match command_path(command) {
            None => AutostartState::Empty,
            Some(path) => {
                if same_exe(&path) {
                    AutostartState::Ok
                } else {
                    AutostartState::Stale(path.display().to_string())
                }
            }
        }
    }

    /// `"C:\app\rasklad.exe" --minimized` (or unquoted) → `C:\app\rasklad.exe`.
    fn command_path(command: &str) -> Option<PathBuf> {
        let s = command.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(rest) = s.strip_prefix('"') {
            let end = rest.find('"')?;
            let p = rest[..end].trim();
            return (!p.is_empty()).then(|| PathBuf::from(p));
        }
        let p = s.split(" --").next().unwrap_or(s).trim();
        (!p.is_empty()).then(|| PathBuf::from(p))
    }

    /// Does `path` resolve to the executable that is running?
    /// A path that no longer exists counts as stale, not equal.
    fn same_exe(path: &std::path::Path) -> bool {
        let Ok(current) = std::env::current_exe() else {
            return false;
        };
        let norm =
            |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        norm(path)
            .to_string_lossy()
            .eq_ignore_ascii_case(&norm(&current).to_string_lossy())
    }

    /// Value of the first `<name>…</name>` element in `xml`.
    fn xml_tag(xml: &str, name: &str) -> Option<String> {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&close)? + start;
        Some(xml[start..end].trim().to_string())
    }

    /// Target of a `.lnk`, read back through the shell.
    fn shortcut_target(lnk: &std::path::Path) -> Option<String> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, STGM_READ,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok()?;
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let persist: IPersistFile = link.cast().ok()?;
            persist
                .Load(&HSTRING::from(lnk.to_string_lossy().as_ref()), STGM_READ)
                .ok()?;
            let mut buf = [0u16; 1024];
            link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..len]))
        }
    }

    /// Disable every autostart mechanism, then enable the requested one.
    /// Errors from disabling are ignored (the entry may simply not exist);
    /// a failure to enable the requested method is reported.
    /// Command line used by every autostart mechanism.
    /// The command line the autostart entry will hold, for the window to show.
    ///
    /// All three mechanisms run the same thing; the scheduled task adds its
    /// own flags to `schtasks`, not to this line.
    pub fn command_preview(start_minimized: bool) -> String {
        match std::env::current_exe() {
            Ok(exe) => launch_command(&exe, start_minimized),
            Err(_) => "rasklad.exe".to_string(),
        }
    }

    fn launch_command(exe: &std::path::Path, start_minimized: bool) -> String {
        if start_minimized {
            format!("\"{}\" --minimized", exe.display())
        } else {
            format!("\"{}\"", exe.display())
        }
    }

    /// The same thing as a bare argument list for a shortcut.
    fn launch_arguments(start_minimized: bool) -> &'static str {
        if start_minimized {
            "--minimized"
        } else {
            ""
        }
    }

    pub fn apply_autostart(method: AutostartMethod, start_minimized: bool) -> io::Result<()> {
        // Build the requested entry first. The old order disabled everything
        // before creating anything, so a failure in the middle left the user
        // with no autostart at all.
        match method {
            AutostartMethod::None => {}
            AutostartMethod::RegistryRun => {
                registry(true, start_minimized)?;
                verify_registry()?;
            }
            AutostartMethod::StartupFolder => {
                startup_folder(true, start_minimized)?;
                match startup_lnk_path() {
                    Ok(p) if p.exists() => {}
                    _ => return Err(io::Error::other("shortcut was not created")),
                }
            }
            AutostartMethod::ScheduledTask => {
                scheduled_task(true, start_minimized)?;
                verify_task()?;
            }
        }

        // The three mechanisms are mutually exclusive. The new entry is in
        // place by now, so the others can go.
        match method {
            AutostartMethod::RegistryRun => {
                let _ = startup_folder(false, start_minimized);
                let _ = scheduled_task(false, start_minimized);
            }
            AutostartMethod::StartupFolder => {
                let _ = registry(false, start_minimized);
                let _ = scheduled_task(false, start_minimized);
            }
            AutostartMethod::ScheduledTask => {
                let _ = registry(false, start_minimized);
                let _ = startup_folder(false, start_minimized);
            }
            AutostartMethod::None => {
                let _ = registry(false, start_minimized);
                let _ = startup_folder(false, start_minimized);
                let _ = scheduled_task(false, start_minimized);
            }
        }
        Ok(())
    }

    /// Read the Run value back to prove the entry is really there.
    fn verify_registry() -> io::Result<()> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
        let value: String = key.get_value("rasklad")?;
        if value.contains("rasklad") {
            Ok(())
        } else {
            Err(io::Error::other("Run entry missing after write"))
        }
    }

    /// Run schtasks without a console window — otherwise a console flash
    /// steals focus for a split second and it lands back on the main
    /// window, hiding the autostart child behind it.
    fn schtasks(args: &[String]) -> io::Result<std::process::Output> {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new("schtasks")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    }

    /// `schtasks /Query` succeeding proves the task exists.
    fn verify_task() -> io::Result<()> {
        let out = schtasks(&["/Query".into(), "/TN".into(), TASK_NAME.into()])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::other("task missing after creation"))
        }
    }

    /// HKCU\Software\Microsoft\Windows\CurrentVersion\Run
    fn registry(enable: bool, start_minimized: bool) -> io::Result<()> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
        if enable {
            let exe = std::env::current_exe()?;
            key.set_value("rasklad", &launch_command(&exe, start_minimized))?;
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
    fn startup_folder(enable: bool, start_minimized: bool) -> io::Result<()> {
        let lnk = startup_lnk_path()?;
        if enable {
            create_shortcut(&lnk, start_minimized)
        } else {
            match fs::remove_file(&lnk) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }

    fn create_shortcut(lnk: &std::path::Path, start_minimized: bool) -> io::Result<()> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

        fn into(e: windows::core::Error) -> io::Error {
            io::Error::other(e)
        }

        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(into)?;
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(into)?;
            let exe = std::env::current_exe()?;
            let exe_s = HSTRING::from(exe.to_string_lossy().as_ref());
            link.SetPath(&exe_s).map_err(into)?;
            link.SetArguments(&HSTRING::from(launch_arguments(start_minimized)))
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

    /// Arguments that register the logon task.
    ///
    /// `/RL HIGHEST` is the whole point: the Task Scheduler service starts
    /// the process with the elevated token, so the app comes up with full
    /// rights and without a UAC prompt at logon. Registry and Startup-folder
    /// entries cannot do that.
    fn task_args(exe: &std::path::Path, start_minimized: bool) -> Vec<String> {
        vec![
            "/Create".into(),
            "/TN".into(),
            TASK_NAME.into(),
            "/SC".into(),
            "ONLOGON".into(),
            "/RL".into(),
            "HIGHEST".into(),
            "/F".into(),
            "/TR".into(),
            launch_command(exe, start_minimized),
        ]
    }

    /// A logon task via schtasks.exe.
    fn scheduled_task(enable: bool, start_minimized: bool) -> io::Result<()> {
        let (args, op): (Vec<String>, &str) = if enable {
            let exe = std::env::current_exe()?;
            (task_args(&exe, start_minimized), "create")
        } else {
            (
                vec![
                    "/Delete".into(),
                    "/TN".into(),
                    TASK_NAME.into(),
                    "/F".into(),
                ],
                "delete",
            )
        };
        let out = schtasks(&args)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "schtasks {} failed: {}",
                op,
                String::from_utf8_lossy(&out.stderr)
            )))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const EXE: &str = "C:/apps/rasklad.exe";

        #[test]
        fn task_runs_with_highest_privileges() {
            let joined = task_args(std::path::Path::new(EXE), true).join(" ");
            assert!(joined.contains("/RL HIGHEST"), "{joined}");
            assert!(joined.contains("/SC ONLOGON"), "{joined}");
            assert!(
                joined.contains(&format!("\"{EXE}\" --minimized")),
                "{joined}"
            );
        }

        #[test]
        fn start_minimized_controls_the_argument() {
            let joined = task_args(std::path::Path::new(EXE), false).join(" ");
            assert!(!joined.contains("--minimized"), "{joined}");
            assert_eq!(launch_arguments(false), "");
            assert_eq!(launch_arguments(true), "--minimized");
        }
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_matches_what_will_be_written() {
        // Quoted, because the path usually has spaces in it.
        assert!(command_preview(true).starts_with('"'));
        assert!(command_preview(true).ends_with("--minimized"));
        assert!(!command_preview(false).contains("--minimized"));
    }

    #[test]
    fn serde_defaults() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.hotkey, Hotkey::default());
        assert_eq!(cfg.autostart, AutostartMethod::None);
        assert!(cfg.start_minimized);
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
    fn a_failed_check_is_not_a_broken_entry() {
        // The repair path only runs for broken states; a failed query must
        // not end up there.
        assert!(!AutostartState::Unknown.is_broken());
        assert!(!AutostartState::Unknown.is_stale_but_valid());
        assert!(AutostartState::Unknown.message().is_some());
        assert!(AutostartState::Missing.is_broken());
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
