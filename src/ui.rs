//! egui settings window plus tray/menu event handling. Runs on the main
//! thread inside eframe's winit event loop.

#![cfg(windows)]

use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;

use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, TrayIconEvent};

use crate::assets;
use crate::config::{self, Config};
use crate::hotkey::Hotkey;
use crate::shared::{CaptureResult, Shared};
use crate::worker::{self, menu_id, WorkerCmd};

pub struct App {
    shared: Arc<Shared>,
    cmds: Sender<WorkerCmd>,
    worker: Option<JoinHandle<()>>,
    show_event: HANDLE,
    recording: bool,
}

impl App {
    fn new(
        shared: Arc<Shared>,
        cmds: Sender<WorkerCmd>,
        worker: JoinHandle<()>,
        show_event: HANDLE,
    ) -> Self {
        Self {
            shared,
            cmds,
            worker: Some(worker),
            show_event,
            recording: false,
        }
    }

    fn config(&self) -> Config {
        self.shared.config.lock().unwrap().clone()
    }

    fn set_enabled(&self, enabled: bool) {
        {
            let mut cfg = self.shared.config.lock().unwrap();
            cfg.enabled = enabled;
            let _ = config::save(&cfg);
        }
        let _ = self.cmds.send(WorkerCmd::RefreshMenu);
        worker::notify(&self.shared);
    }

    fn set_hotkey(&self, hotkey: Hotkey) {
        let cfg = self.shared.config.lock().unwrap();
        let mut next = cfg.clone();
        drop(cfg);
        next.hotkey = hotkey;
        let _ = config::save(&next);
        *self.shared.config.lock().unwrap() = next;
    }

    fn set_autostart(&self, on: bool) {
        if config::set_autostart(on).is_ok() {
            let mut cfg = self.shared.config.lock().unwrap();
            cfg.autostart = on;
            let _ = config::save(&cfg);
        }
    }

    fn show(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn quit(&mut self) {
        let _ = self.cmds.send(WorkerCmd::Shutdown);
        worker::notify(&self.shared);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
        std::process::exit(0);
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        // Tray icon clicks: left click / double click opens settings.
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            let left = match &ev {
                TrayIconEvent::Click { button, .. } => *button == MouseButton::Left,
                TrayIconEvent::DoubleClick { button, .. } => *button == MouseButton::Left,
                _ => false,
            };
            if left {
                self.show(ctx);
            }
        }

        // Tray menu.
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            match ev.id.0.as_str() {
                menu_id::SETTINGS => self.show(ctx),
                menu_id::ENABLED => {
                    let on = !self.config().enabled;
                    self.set_enabled(on);
                }
                menu_id::QUIT => self.quit(),
                _ => {}
            }
        }

        // Second instance asked us to show the window.
        unsafe {
            if WaitForSingleObject(self.show_event, 0) == WAIT_OBJECT_0 {
                let _ = ResetEvent(self.show_event);
                self.show(ctx);
            }
        }

        // Hotkey capture finished in the hook.
        if let Some(res) = self.shared.capture.lock().unwrap().take() {
            self.recording = false;
            if let CaptureResult::Captured(hk) = res {
                self.set_hotkey(hk);
            }
        }

        // Window close button hides to tray instead of exiting.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events(ctx);

        let cfg = self.config();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("capswitch");
            ui.label("Switch keyboard layout with a custom hotkey.");
            ui.add_space(8.0);

            let mut enabled = cfg.enabled;
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                self.set_enabled(enabled);
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Hotkey:");
                ui.monospace(cfg.hotkey.display());
                if self.recording {
                    ui.label("press keys… (Esc to cancel)");
                } else {
                    if ui.button("Record").clicked() {
                        self.recording = true;
                        self.shared
                            .capturing
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    if ui.button("Reset").clicked() {
                        self.set_hotkey(Hotkey::default());
                    }
                }
            });

            ui.add_space(4.0);
            let mut autostart = cfg.autostart;
            if ui
                .checkbox(&mut autostart, "Run at Windows startup")
                .changed()
            {
                self.set_autostart(autostart);
            }

            ui.add_space(8.0);
            ui.separator();
            ui.small("Lives in the system tray. Closing this window keeps it running.");
            ui.small("Default: Caps Lock switches layout; Shift+Caps still toggles caps.");
        });

        // Keep polling tray/menu/hook events even while the window is hidden.
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

pub fn run(
    shared: Arc<Shared>,
    cmds: Sender<WorkerCmd>,
    worker: JoinHandle<()>,
    show_event: HANDLE,
    visible: bool,
) {
    let (rgba, w, h) = assets::icon_rgba();
    let icon = egui::IconData {
        rgba,
        width: w,
        height: h,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([330.0, 175.0])
            .with_resizable(false)
            .with_visible(visible)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };

    let app = App::new(shared, cmds, worker, show_event);
    let _ = eframe::run_native("capswitch", options, Box::new(move |_cc| Ok(Box::new(app))));
}
