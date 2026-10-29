//! Worker thread: owns the low-level keyboard hook and the tray icon, and
//! pumps a Win32 message loop that both need. The GUI thread talks to it
//! through a command channel plus PostThreadMessage wakeups.

#![cfg(windows)]

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;

use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::hook;
use crate::shared::Shared;

pub enum WorkerCmd {
    /// Re-read shared config and refresh the "Enabled" checkmark.
    RefreshMenu,
    Shutdown,
}

pub const WM_WAKEUP: u32 = WM_APP + 1;

pub mod menu_id {
    pub const ENABLED: &str = "rasklad.enabled";
    pub const QUIT: &str = "rasklad.quit";
}

/// Toggle the master switch from either the GUI checkbox or the tray menu,
/// then refresh the menu checkmark.
pub fn set_enabled(shared: &Shared, cmds: &Sender<WorkerCmd>, on: bool) {
    {
        let mut cfg = shared.config.lock().unwrap();
        cfg.enabled = on;
        let _ = crate::config::save(&cfg);
    }
    let _ = cmds.send(WorkerCmd::RefreshMenu);
    notify(shared);
}

/// Wake the worker's message loop so it drains the command channel.
pub fn notify(shared: &Shared) {
    unsafe {
        let tid = shared.worker_tid.load(Ordering::Relaxed);
        if tid != 0 {
            let _ = PostThreadMessageW(tid, WM_WAKEUP, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn run(
    shared: Arc<Shared>,
    cmds: Receiver<WorkerCmd>,
    tid_tx: Sender<u32>,
    icon_rgba: (Vec<u8>, u32, u32),
) {
    unsafe {
        let enabled = shared.config.lock().unwrap().enabled;
        let menu = Menu::new();
        let mi_enabled = CheckMenuItem::with_id(menu_id::ENABLED, "Enabled", true, enabled, None);
        let mi_quit = MenuItem::with_id(menu_id::QUIT, "Quit", true, None);
        let _ = menu.append_items(&[&mi_enabled, &PredefinedMenuItem::separator(), &mi_quit]);

        let _tray = add_tray_icon(icon_rgba.clone(), menu, enabled);

        let hhk = match hook::install(shared.clone()) {
            Ok(hhk) => hhk,
            Err(e) => {
                crate::diag::log(&format!("keyboard hook failed: {e:?}"));
                return;
            }
        };
        let _ = tid_tx.send(GetCurrentThreadId());

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_WAKEUP {
                while let Ok(cmd) = cmds.try_recv() {
                    match cmd {
                        WorkerCmd::RefreshMenu => {
                            let on = shared.config.lock().unwrap().enabled;
                            mi_enabled.set_checked(on);
                            // The icon is the only always-visible answer to
                            // "is this thing on", so it follows the switch.
                            if let Some(tray) = _tray.as_ref() {
                                if let Some(icon) = tray_icon(&icon_rgba, on) {
                                    let _ = tray.set_icon(Some(icon));
                                }
                                let _ = tray.set_tooltip(Some(tooltip(on)));
                            }
                        }
                        WorkerCmd::Shutdown => PostQuitMessage(0),
                    }
                }
            } else {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        hook::uninstall(hhk);
    }
}

/// The icon as it should look for the current state.
fn tray_icon(icon_rgba: &(Vec<u8>, u32, u32), enabled: bool) -> Option<Icon> {
    let (rgba, width, height) = icon_rgba;
    let data = if enabled {
        rgba.clone()
    } else {
        crate::assets::dimmed(rgba)
    };
    Icon::from_rgba(data, *width, *height).ok()
}

fn tooltip(enabled: bool) -> &'static str {
    if enabled {
        "rasklad"
    } else {
        "rasklad - hotkey off"
    }
}

/// Add the notification-area icon, waiting for the shell to be ready.
///
/// Started by the logon task the process can come up before Explorer has
/// created the notification area; `Shell_NotifyIcon` then fails with E_FAIL.
/// That is a matter of a second or two, so retry instead of dying: a panic
/// here greeted the user with a crash dialog on every logon.
fn add_tray_icon(icon_rgba: (Vec<u8>, u32, u32), menu: Menu, enabled: bool) -> Option<TrayIcon> {
    for attempt in 1..=80 {
        let icon = match tray_icon(&icon_rgba, enabled) {
            Some(icon) => icon,
            None => {
                crate::diag::log("tray icon data rejected");
                return None;
            }
        };
        match TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_tooltip(tooltip(enabled))
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(tray) => {
                if attempt > 1 {
                    crate::diag::log(&format!("tray icon added on attempt {attempt}"));
                }
                return Some(tray);
            }
            Err(e) => {
                if attempt == 1 || attempt % 20 == 0 {
                    crate::diag::log(&format!("tray icon attempt {attempt} failed: {e:?}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
    crate::diag::log("tray icon could not be added, continuing without it");
    None
}
