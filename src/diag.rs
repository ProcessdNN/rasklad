//! Diagnostic log, panic reporting and the "copy diagnostics" report.
//!
//! Logging is off by default. A tray tool that writes a file all day is a
//! liability, so it is turned on for one run with `--debug`, or at runtime
//! from the Advanced window, and is off again after a restart.
//!
//! Nothing here records what you type. Entries are events: start, config
//! changes, hotkey fires, switch results, autostart work and panics.

#![cfg(windows)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::core::HSTRING;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use crate::config::{self, Config};

/// Rotate once the file grows past this. One previous file is kept.
const MAX_BYTES: u64 = 1024 * 1024;

/// How many lines of the log end up in the diagnostics report.
const REPORT_LINES: usize = 200;

static ENABLED: AtomicBool = AtomicBool::new(false);
static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Where the log goes. `RASKLAD_LOG` overrides the location, which is useful
/// for support runs and CI; otherwise it sits in the local app data folder.
fn path() -> Option<&'static PathBuf> {
    PATH.get_or_init(|| {
        if let Some(custom) = std::env::var_os("RASKLAD_LOG") {
            return Some(PathBuf::from(custom));
        }
        let base = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(base).join("rasklad").join("rasklad.log"))
    })
    .as_ref()
}

/// Called once at startup with the value of `--debug`.
///
/// `RASKLAD_LOG` also turns logging on: setting it means someone wants a log
/// for a support run, and a path with nothing in it would only be confusing.
pub fn init(on: bool) {
    let on = on || std::env::var_os("RASKLAD_LOG").is_some();
    ENABLED.store(on, Ordering::Relaxed);
    if on {
        log("debug log enabled");
    }
}

/// Folder holding the log file, for the button in Advanced.
pub fn log_dir() -> Option<PathBuf> {
    path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Turn logging on or off at runtime, remembering the change in the log.
pub fn enable(on: bool) {
    if on {
        ENABLED.store(true, Ordering::Relaxed);
        log("debug log enabled");
    } else {
        log("debug log disabled");
        ENABLED.store(false, Ordering::Relaxed);
    }
}

/// Record an event. Does nothing while logging is off.
pub fn log(msg: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    write_line(msg);
}

/// Record an event even while logging is off. Used by the panic hook: after
/// a crash the log is the only thing the user can send.
fn write_line(msg: &str) {
    let Some(file) = path() else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    rotate_if_needed(file);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let _ = writeln!(f, "{} {}", timestamp(), msg);
    }
}

fn rotate_if_needed(file: &Path) {
    let Ok(meta) = std::fs::metadata(file) else {
        return;
    };
    if meta.len() < MAX_BYTES {
        return;
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".1");
    let previous = file.with_file_name(name);
    let _ = std::fs::remove_file(&previous);
    let _ = std::fs::rename(file, previous);
}

/// Write panics to the log and tell the user where to find it. A tray app
/// that disappears without a word is impossible to report.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        write_line(&format!("PANIC {info}"));
        write_line(&format!(
            "backtrace:\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        let where_to = path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(no log path available)".to_string());
        let body =
            format!("Rasklad stopped unexpectedly.\n\n{info}\n\nThe log was saved to:\n{where_to}");
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(body),
                &HSTRING::from("Rasklad"),
                MB_OK | MB_ICONERROR,
            );
        }
    }));
}

/// The text the "Copy diagnostics" button puts on the clipboard.
pub fn report(cfg: &Config) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Rasklad {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "Windows: {}", windows_version());
    let _ = writeln!(out, "Architecture: {}", std::env::consts::ARCH);
    let _ = writeln!(
        out,
        "Elevated: {}",
        if crate::elevation::is_elevated() {
            "yes"
        } else {
            "no"
        }
    );
    let _ = writeln!(out, "Enabled: {}", cfg.enabled);
    let _ = writeln!(out, "Hotkey: {}", cfg.hotkey.display());
    let _ = writeln!(out, "Caps combo: {}", cfg.caps_combo.title());
    let _ = writeln!(out, "Autostart: {}", cfg.autostart.title());
    let _ = writeln!(out, "Start minimized: {}", cfg.start_minimized);
    let _ = writeln!(
        out,
        "Autostart state: {}",
        config::autostart_state(cfg.autostart)
            .message()
            .unwrap_or_else(|| "ok".into())
    );
    match path() {
        Some(p) => {
            let _ = writeln!(
                out,
                "Log file: {} ({})",
                p.display(),
                if is_enabled() { "on" } else { "off" }
            );
        }
        None => {
            let _ = writeln!(out, "Log file: (none)");
        }
    }

    let _ = writeln!(out, "\n--- last {REPORT_LINES} log lines ---");
    match tail(REPORT_LINES) {
        Some(lines) if !lines.is_empty() => out.push_str(&lines),
        _ => out.push_str("(empty)\n"),
    }
    out
}

fn tail(count: usize) -> Option<String> {
    let file = path()?;
    let text = std::fs::read_to_string(file).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(count);
    let mut out = lines[start..].join("\n");
    out.push('\n');
    Some(out)
}

fn windows_version() -> String {
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let key = winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let Ok(key) = key else {
        return "unknown".to_string();
    };
    let product: String = key.get_value("ProductName").unwrap_or_default();
    let display: String = key.get_value("DisplayVersion").unwrap_or_default();
    let build: String = key.get_value("CurrentBuild").unwrap_or_default();
    let revision: u32 = key.get_value("UBR").unwrap_or(0);
    format!("{product} {display} (build {build}.{revision})")
}

fn timestamp() -> String {
    let st = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_mentions_the_version_and_config() {
        let text = report(&Config::default());
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("Hotkey:"));
        assert!(text.contains("Start minimized:"));
    }
}
