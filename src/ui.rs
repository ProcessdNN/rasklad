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
use crate::elevation;
use crate::hotkey::Hotkey;
use crate::shared::{CaptureResult, Shared};
use crate::worker::{self, menu_id, WorkerCmd};
use crate::winshow;

/// The standard egui toggle switch (as in the egui demo): an animated
/// pill with a sliding knob.
fn toggle_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool(response.id, *on);
        let visuals = ui.style().interact_selectable(&response, *on);
        let rect = rect.expand(visuals.expansion);
        let radius = 0.5 * rect.height();
        ui.painter().rect(
            rect,
            radius,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = egui::pos2(circle_x, rect.center().y);
        ui.painter()
            .circle(center, 0.75 * radius, visuals.bg_fill, visuals.fg_stroke);
    }
    response
}

/// Hyperlink-styled label: light gray, easing to white on hover.
fn link_label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let font_id = egui::FontId::proportional(13.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE);
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    let t = ui.ctx().animate_value_with_time(
        response.id,
        if response.hovered() { 1.0 } else { 0.0 },
        0.15,
    );
    let gray = (175.0 + (245.0 - 175.0) * t) as u8;
    let color = egui::Color32::from_gray(gray);
    if ui.is_rect_visible(rect) {
        ui.painter()
            .text(rect.min, egui::Align2::LEFT_TOP, text, font_id, color);
        let y = rect.bottom() - 1.0;
        ui.painter().line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0_f32, color),
        );
    }
    response
}

pub struct App {
    shared: Arc<Shared>,
    cmds: Sender<WorkerCmd>,
    worker: Option<JoinHandle<()>>,
    icon: Arc<egui::IconData>,
    elevated: bool,
    recording: bool,
    admin_status: Option<String>,
    autostart_open: bool,
    autostart_sel: AutostartMethod,
    autostart_status: Option<String>,
}

impl App {
    fn new(
        shared: Arc<Shared>,
        cmds: Sender<WorkerCmd>,
        worker: JoinHandle<()>,
        icon: Arc<egui::IconData>,
    ) -> Self {
        Self {
            shared,
            cmds,
            worker: Some(worker),
            icon,
            elevated: elevation::is_elevated(),
            recording: false,
            admin_status: None,
            autostart_open: false,
            autostart_sel: AutostartMethod::RegistryRun,
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

    fn autostart_viewport(&mut self, ctx: &egui::Context, cfg: &Config) {
        // Pop next to the main window instead of wherever the WM puts it.
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Autostart")
            .with_inner_size([300.0, 110.0])
            .with_resizable(false)
            .with_icon(self.icon.clone());
        if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
            let c = outer.center();
            builder = builder.with_position(egui::pos2(c.x - 150.0, c.y - 55.0));
        }
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("autostart-window"),
            builder,
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.autostart_open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(2.0);

                    let sel = if cfg.autostart != AutostartMethod::None {
                        cfg.autostart
                    } else {
                        self.autostart_sel
                    };
                    let mut on = cfg.autostart != AutostartMethod::None;

                    ui.horizontal(|ui| {
                        ui.label("Autostart");
                        if toggle_ui(ui, &mut on).changed() {
                            self.set_autostart(if on { sel } else { AutostartMethod::None });
                        }
                    });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Method:");
                        ui.add_enabled_ui(on, |ui| {
                            egui::ComboBox::from_id_salt("autostart-method")
                                .selected_text(sel.title())
                                .show_ui(ui, |ui| {
                                    for m in AutostartMethod::ALL {
                                        if ui.selectable_label(sel == m, m.title()).clicked() {
                                            self.autostart_sel = m;
                                            if on {
                                                self.set_autostart(m);
                                            }
                                        }
                                    }
                                });
                        });
                    });

                    if let Some(err) = &self.autostart_status {
                        ui.add_space(6.0);
                        ui.colored_label(egui::Color32::LIGHT_RED, err.as_str());
                    }
                });
            },
        );
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
                    // U+22EF is missing from egui's bundled font — use
                    // the plain ellipsis so the button doesn't draw tofu.
                    ui.menu_button("…", |ui| {
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
            ui.horizontal(|ui| {
                if toggle_ui(ui, &mut enabled).changed() {
                    worker::set_enabled(&self.shared, &self.cmds, enabled);
                }
                ui.label(if enabled { "Enabled" } else { "Disabled" });
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Hotkey:");
                ui.monospace(cfg.hotkey.display());
                if self.recording {
                    ui.label("press keys…");
                    if ui.button("Cancel").clicked() {
                        self.recording = false;
                        self.shared.capturing.store(false, Ordering::SeqCst);
                    }
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
            if !self.elevated {
                ui.label("Not running as administrator — layout switching");
                ui.horizontal(|ui| {
                    ui.label("won't work in elevated windows.");
                    if link_label(ui, "relaunch as admin").clicked() {
                        if elevation::relaunch_elevated() {
                            self.quit();
                        } else {
                            self.admin_status = Some(
                                "relaunch failed — UAC prompt was declined or blocked"
                                    .to_string(),
                            );
                        }
                    }
                });
                if let Some(err) = &self.admin_status {
                    ui.colored_label(egui::Color32::LIGHT_RED, err.as_str());
                }
            }
        });

        if self.autostart_open {
            self.autostart_viewport(ctx, &cfg);
        }

        // Repaint periodically so capture results and toggle state stay
        // fresh; tray/menu events additionally trigger request_repaint().
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

/// Drains tray-icon, tray-menu and second-instance events and reacts
/// immediately. Besides egui viewport commands it also raises the native
/// window directly — Windows refuses SetForegroundWindow to background
/// processes, so without this the window only flashes on the taskbar.
fn spawn_events(
    ctx: egui::Context,
    shared: Arc<Shared>,
    cmds: Sender<WorkerCmd>,
    show_event: usize,
) {
    let show = |ctx: &egui::Context| {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        winshow::raise_main_window();
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
    let icon = Arc::new(egui::IconData {
        rgba,
        width: w,
        height: h,
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([340.0, 200.0])
            .with_resizable(false)
            .with_visible(visible)
            .with_icon(icon.clone()),
        ..Default::default()
    };

    let app_cmds = cmds.clone();
    let _ = eframe::run_native(
        "rasklad",
        options,
        Box::new(move |cc| {
            spawn_events(cc.egui_ctx.clone(), shared.clone(), app_cmds, show_event.0 as usize);
            Ok(Box::new(App::new(shared, cmds, worker, icon)))
        }),
    );
}
