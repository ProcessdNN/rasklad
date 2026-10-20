//! egui settings window, plus a dedicated event thread that owns the
//! tray/menu event receivers so clicks are handled instantly even while
//! the window is hidden (egui only repaints on demand).

#![cfg(windows)]

use std::sync::atomic::Ordering;
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
use crate::config::{self, AutostartMethod, Config};
use crate::hotkey::Hotkey;
use crate::shared::{CaptureResult, Shared};
use crate::worker::{self, menu_id, WorkerCmd};

pub struct App {
    shared: Arc<Shared>,
    cmds: Sender<WorkerCmd>,
    worker: Option<JoinHandle<()>>,
    recording: bool,
    autostart_open: bool,
    autostart_status: Option<String>,
}

impl App {
    fn new(shared: Arc<Shared>, cmds: Sender<WorkerCmd>, worker: JoinHandle<()>) -> Self {
        Self {
            shared,
            cmds,
            worker: Some(worker),
            recording: false,
            autostart_open: false,
            autostart_status: None,
        }
    }

    fn config(&self) -> Config {
        self.shared.config.lock().unwrap().clone()
    }

    fn set_hotkey(&self, hotkey: Hotkey) {
        let next = {
            let cfg = self.shared.config.lock().unwrap();
            let mut next = cfg.clone();
            next.hotkey = hotkey;
            next
        };
        let _ = config::save(&next);
        *self.shared.config.lock().unwrap() = next;
    }

    fn set_autostart(&mut self, method: AutostartMethod) {
        match config::apply_autostart(method) {
            Ok(()) => {
                let mut cfg = self.shared.config.lock().unwrap();
                cfg.autostart = method;
                let _ = config::save(&cfg);
                self.autostart_status = None;
            }
            Err(e) => self.autostart_status = Some(e.to_string()),
        }
    }

    fn quit(&mut self) {
        let _ = self.cmds.send(WorkerCmd::Shutdown);
        worker::notify(&self.shared);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
        std::process::exit(0);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        let cfg = self.config();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("rasklad");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⋯", |ui| {
                        if ui.button("Autostart…").clicked() {
                            self.autostart_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Exit").clicked() {
                            self.quit();
                        }
                    });
                });
            });
            ui.label("Switch keyboard layout with a custom hotkey.");
            ui.add_space(8.0);

            let mut enabled = cfg.enabled;
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                worker::set_enabled(&self.shared, &self.cmds, enabled);
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
                        self.shared.capturing.store(true, Ordering::SeqCst);
                    }
                    if ui.button("Reset").clicked() {
                        self.set_hotkey(Hotkey::default());
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label("Lives in the system tray. Closing this window keeps it running.");
            ui.label("Default: Caps Lock switches layout; Shift+Caps still toggles caps.");
        });

        // Autostart settings sub-window.
        if self.autostart_open {
            let method = cfg.autostart;
            let mut open = self.autostart_open;
            egui::Window::new("Autostart")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Run rasklad when you log in — pick one method:");
                    egui::ComboBox::from_id_salt("autostart-method")
                        .selected_text(method.title())
                        .show_ui(ui, |ui| {
                            for m in AutostartMethod::ALL {
                                if ui.selectable_label(method == m, m.title()).clicked() {
                                    self.set_autostart(m);
                                }
                            }
                        });
                    ui.add_space(8.0);
                    ui.label(format!("Active: {}", method.title()));
                    if ui.button("Turn off").clicked() {
                        self.set_autostart(AutostartMethod::None);
                    }
                    if let Some(err) = &self.autostart_status {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                });
            self.autostart_open = open;
        }

        // Repaint periodically so capture results and checkbox state stay
        // fresh; tray/menu events additionally trigger request_repaint().
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

/// Drains tray-icon, tray-menu and second-instance events and reacts
/// immediately: viewport commands are thread-safe, so showing the window
/// does not depend on egui's repaint cycle.
fn spawn_events(
    ctx: egui::Context,
    shared: Arc<Shared>,
    cmds: Sender<WorkerCmd>,
    show_event: usize,
) {
    let show = |ctx: &egui::Context| {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    };
    std::thread::spawn(move || loop {
        let show_event = HANDLE(show_event as *mut _);
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            let left = matches!(
                ev,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            );
            if left {
                show(&ctx);
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            match ev.id.0.as_str() {
                menu_id::ENABLED => {
                    let on = !shared.config.lock().unwrap().enabled;
                    worker::set_enabled(&shared, &cmds, on);
                    ctx.request_repaint();
                }
                menu_id::QUIT => {
                    let _ = cmds.send(WorkerCmd::Shutdown);
                    worker::notify(&shared);
                    // Give the worker a moment to unhook and drop the tray
                    // icon before the process dies.
                    std::thread::sleep(Duration::from_millis(250));
                    std::process::exit(0);
                }
                _ => {}
            }
        }
        unsafe {
            if WaitForSingleObject(show_event, 0) == WAIT_OBJECT_0 {
                let _ = ResetEvent(show_event);
                show(&ctx);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    });
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

    let app_cmds = cmds.clone();
    let _ = eframe::run_native(
        "rasklad",
        options,
        Box::new(move |cc| {
            spawn_events(cc.egui_ctx.clone(), shared.clone(), app_cmds, show_event.0 as usize);
            Ok(Box::new(App::new(shared, cmds, worker)))
        }),
    );
}
