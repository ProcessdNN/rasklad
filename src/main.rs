#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod config;
#[cfg(windows)]
mod diag;
mod hotkey;
mod shared;
mod vk;

#[cfg(windows)]
mod elevation;
#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod switcher;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod winshow;
#[cfg(windows)]
mod worker;

/// Command line switches. Parsed in one place so the flag handling can be
/// tested without starting the app.
#[derive(Debug, Default, PartialEq, Eq)]
struct Launch {
    /// `--minimized`: stay in the tray, used by the autostart entry.
    minimized: bool,
    /// `--debug`: write the diagnostic log for this run.
    debug: bool,
    /// `--elevated`: started by the "relaunch as admin" link, wait for the
    /// previous instance to release the mutex.
    wait_primary: bool,
    /// `--apply-autostart <key>`: one-shot privileged helper.
    apply_autostart: Option<String>,
    /// `--version`: print the version and exit.
    version: bool,
    /// `--help`: print the usage and exit.
    help: bool,
}

fn parse_launch<I, S>(args: I) -> Launch
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let apply_autostart = args
        .iter()
        .position(|a| a == "--apply-autostart")
        .and_then(|i| args.get(i + 1))
        .cloned();
    Launch {
        minimized: has("--minimized"),
        debug: has("--debug"),
        wait_primary: has("--elevated"),
        apply_autostart,
        version: has("--version") || has("-V"),
        help: has("--help") || has("-h"),
    }
}

const USAGE: &str = concat!(
    "rasklad ",
    env!("CARGO_PKG_VERSION"),
    " - keyboard layout switcher for Windows\n\
     \n\
     Usage: rasklad [options]\n\
     \n\
     \x20 --minimized          start in the tray without opening the window\n\
     \x20 --debug              write the diagnostic log for this run\n\
     \x20 --version            print the version and exit\n\
     \x20 --help               print this text and exit\n\
     \n\
     Without arguments the settings window opens. Autostart passes --minimized.\n"
);

/// Write to the console this process was started from.
///
/// The exe is built for the windows subsystem, so it has no console of its
/// own and `println!` would go nowhere. Attaching to the parent console makes
/// `rasklad --version` work from cmd; started from Explorer there is no
/// parent console, so the text goes into a message box instead.
fn print_out(text: &str) {
    use std::io::Write as _;
    use windows::Win32::Storage::FileSystem::{
        GetFileType, FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE,
    };
    use windows::Win32::System::Console::{
        AttachConsole, GetConsoleMode, GetStdHandle, ATTACH_PARENT_PROCESS, CONSOLE_MODE,
        STD_OUTPUT_HANDLE,
    };

    // Writing to a handle that does not exist reports success on Windows, and
    // a process started without a console is often handed NUL, which looks
    // like a character device. Ask what the handle really is: a file or a
    // pipe is a redirection and works, a character device only counts when it
    // answers as a console.
    let (usable, is_console) = unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE).unwrap_or_default();
        match GetFileType(handle) {
            FILE_TYPE_DISK | FILE_TYPE_PIPE => (true, false),
            FILE_TYPE_CHAR => {
                let mut mode = CONSOLE_MODE::default();
                let console = GetConsoleMode(handle, &mut mode).is_ok();
                (console, console)
            }
            _ => (false, false),
        }
    };
    if usable {
        // A console is a shell that has already printed its prompt: the
        // command returns before this process writes anything, so start on a
        // fresh line instead of gluing the text to the prompt.
        let payload = if is_console {
            format!("\r\n{text}")
        } else {
            text.to_string()
        };
        let mut stdout = std::io::stdout();
        if stdout
            .write_all(payload.as_bytes())
            .and_then(|_| stdout.flush())
            .is_ok()
        {
            return;
        }
    }

    // Otherwise borrow the console this was started from.
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS).is_ok() };
    if attached {
        if let Ok(mut out) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            let _ = out.write_all(text.as_bytes());
            return;
        }
    }
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            &windows::core::HSTRING::from(text),
            &windows::core::HSTRING::from("Rasklad"),
            windows::Win32::UI::WindowsAndMessaging::MB_OK,
        );
    }
}

#[cfg(windows)]
fn main() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::channel;
    use std::sync::Arc;

    let launch = parse_launch(std::env::args());
    // Autostart registers us with --minimized: stay in the tray.
    // A manual launch opens the settings window instead.
    let minimized = launch.minimized;
    let wait_primary = launch.wait_primary;
    let debug = launch.debug;

    if launch.help {
        print_out(USAGE);
        return;
    }
    if launch.version {
        print_out(&format!("rasklad {}\n", env!("CARGO_PKG_VERSION")));
        return;
    }

    diag::install_panic_hook();
    diag::init(debug);

    // One-shot privileged helper, started by the Autostart window when the
    // selected method needs administrator rights. It applies the entry from
    // the saved config and exits; the running instance is left alone.
    if let Some(key) = launch.apply_autostart.as_deref() {
        let method = config::AutostartMethod::from_key(key);
        let ok = match method {
            Some(m) => config::apply_autostart(m, config::load().start_minimized).is_ok(),
            None => false,
        };
        diag::log(&format!("helper: apply autostart {method:?} ok={ok}"));
        std::process::exit(if ok { 0 } else { 1 });
    }

    let show_event = match if wait_primary {
        instance::acquire_primary_wait(10_000)
    } else {
        instance::acquire()
    } {
        instance::Instance::Secondary => return,
        instance::Instance::Primary { show, .. } => show,
    };

    let shared = Arc::new(shared::Shared::new(config::load()));
    diag::log(&format!(
        "start: {} elevated={} minimized={}",
        env!("CARGO_PKG_VERSION"),
        elevation::is_elevated(),
        minimized
    ));

    let (cmd_tx, cmd_rx) = channel();
    let (tid_tx, tid_rx) = channel();
    // The tray draws a small icon; ask Windows which size that is (16 at
    // 100% scaling, 20/24/32 on scaled displays) so it is not resampled.
    let icon = assets::icon_rgba(unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON,
        )
        .clamp(16, 64) as u32
    });
    let worker_shared = shared.clone();
    let worker = std::thread::spawn(move || worker::run(worker_shared, cmd_rx, tid_tx, icon));

    if let Ok(tid) = tid_rx.recv() {
        shared.worker_tid.store(tid, Ordering::SeqCst);
    } else {
        // Worker failed to start (hook/tray init error) — nothing to run.
        eprintln!("rasklad: failed to initialize hook/tray");
        std::process::exit(1);
    }

    ui::run(
        shared.clone(),
        cmd_tx.clone(),
        worker,
        show_event,
        !minimized,
    );

    // eframe returned: make sure the worker loop is torn down too.
    let _ = cmd_tx.send(worker::WorkerCmd::Shutdown);
    worker::notify(&shared);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("rasklad is a Windows-only application.");
}

#[cfg(test)]
mod tests {
    use super::parse_launch;

    #[test]
    fn parses_the_autostart_switches() {
        let launch = parse_launch(["rasklad.exe", "--minimized"]);
        assert!(launch.minimized);
        assert!(!launch.debug);
        assert!(launch.apply_autostart.is_none());
    }

    #[test]
    fn parses_debug_and_elevated() {
        let launch = parse_launch(["rasklad.exe", "--debug", "--elevated"]);
        assert!(launch.debug);
        assert!(launch.wait_primary);
        assert!(!launch.minimized);
    }

    #[test]
    fn parses_version_and_help() {
        assert!(parse_launch(["rasklad.exe", "--version"]).version);
        assert!(parse_launch(["rasklad.exe", "-V"]).version);
        assert!(parse_launch(["rasklad.exe", "--help"]).help);
        assert!(parse_launch(["rasklad.exe", "-h"]).help);
        let plain = parse_launch(["rasklad.exe"]);
        assert!(!plain.version && !plain.help);
    }

    #[test]
    fn parses_the_helper_key() {
        let launch = parse_launch(["rasklad.exe", "--apply-autostart", "task"]);
        assert_eq!(launch.apply_autostart.as_deref(), Some("task"));
        // A missing value must not be mistaken for a method.
        assert!(parse_launch(["rasklad.exe", "--apply-autostart"])
            .apply_autostart
            .is_none());
    }
}
