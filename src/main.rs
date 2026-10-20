#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod config;
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

    // Autostart registers us with --minimized: stay in the tray.
    // A manual launch opens the settings window instead.
    let minimized = std::env::args().any(|a| a == "--minimized");
    // Started by the "relaunch as admin" link: the previous process is on
    // its way out — wait for it to release the single-instance mutex.
    let wait_primary = std::env::args().any(|a| a == "--elevated");

    let show_event = match if wait_primary {
        instance::acquire_primary_wait(10_000)
    } else {
        instance::acquire()
    } {
        instance::Instance::Secondary => return,
        instance::Instance::Primary { show, .. } => show,
    };

    let shared = Arc::new(shared::Shared::new(config::load()));

    let (cmd_tx, cmd_rx) = channel();
    let (tid_tx, tid_rx) = channel();
    let icon = assets::icon_rgba();
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
