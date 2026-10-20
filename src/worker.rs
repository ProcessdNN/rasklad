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
use tray_icon::{Icon, TrayIconBuilder};

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
        let _ = menu.append_items(&[
            &mi_enabled,
            &PredefinedMenuItem::separator(),
            &mi_quit,
        ]);

        let (rgba, w, h) = icon_rgba;
        let icon = Icon::from_rgba(rgba, w, h).expect("tray icon rgba");
        let _tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("rasklad")
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
            .expect("tray icon");

        let hhk = hook::install(shared.clone()).expect("keyboard hook");
        let _ = tid_tx.send(GetCurrentThreadId());

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_WAKEUP {
                while let Ok(cmd) = cmds.try_recv() {
                    match cmd {
                        WorkerCmd::RefreshMenu => {
                            let on = shared.config.lock().unwrap().enabled;
                            mi_enabled.set_checked(on);
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
