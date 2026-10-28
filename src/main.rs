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

#[cfg(windows)]
fn main() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::channel;
    use std::sync::Arc;

    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    // Autostart registers us with --minimized: stay in the tray.
    // A manual launch opens the settings window instead.
    let minimized = has("--minimized");
    // Started by the "relaunch as admin" link: the previous process is on
    // its way out — wait for it to release the single-instance mutex.
    let wait_primary = has("--elevated");
    // Diagnostic log for this run only; the Advanced window toggles the same
    // thing at runtime.
    let debug = has("--debug");

    diag::install_panic_hook();
    diag::init(debug);

    // One-shot privileged helper, started by the Autostart window when the
    // selected method needs administrator rights. It applies the entry from
    // the saved config and exits; the running instance is left alone.
    if let Some(pos) = args.iter().position(|a| a == "--apply-autostart") {
        let method = args.get(pos + 1).and_then(|k| config::AutostartMethod::from_key(k));
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
        "start: {} elevated={}",
        env!("CARGO_PKG_VERSION"),
        elevation::is_elevated()
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

    ui::run(shared.clone(), cmd_tx.clone(), worker, show_event, !minimized);

    // eframe returned: make sure the worker loop is torn down too.
    let _ = cmd_tx.send(worker::WorkerCmd::Shutdown);
    worker::notify(&shared);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("rasklad is a Windows-only application.");
}
