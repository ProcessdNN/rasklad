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
use crate::config::{self, AutostartMethod, CapsCombo, Config};
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

/// Entries of the ⋯ menu. `Exit` is drawn after a separator.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Autostart,
    Advanced,
    Exit,
}

const MENU_ITEMS: [(MenuAction, &str); 3] = [
    (MenuAction::Autostart, "Autostart…"),
    (MenuAction::Advanced, "Advanced…"),
    (MenuAction::Exit, "Exit"),
];

const MENU_ITEM_H: f32 = 24.0;
const MENU_SEP_H: f32 = 9.0;
const MENU_TEXT_LEFT: f32 = 9.0;
/// How far the menu travels while appearing, in points.
const MENU_SLIDE: f32 = 5.0;

fn menu_font() -> egui::FontId {
    egui::FontId::proportional(13.5)
}

/// A child window (Autostart / Advanced) that is only revealed once the OS
/// window really has the size we asked for.
///
/// winit creates a new window at the system default size (1440x789 here) and
/// applies `with_inner_size` a frame or two later, so a window that is shown
/// at creation flashes as a big empty rectangle first. Creating it hidden
/// and showing it once the size matches removes that flash.
#[derive(Default)]
struct ChildWindow {
    shown: bool,
    polls: u32,
}

impl ChildWindow {
    fn reset(&mut self) {
        self.shown = false;
        self.polls = 0;
    }

    /// True when the caller should paint (and the window is ready to show).
    fn ready(&mut self, ctx: &egui::Context, size: egui::Vec2) -> bool {
        if self.shown {
            return true;
        }
        self.polls += 1;
        let sized = ctx
            .input(|i| i.viewport().inner_rect)
            .map_or(false, |r| (r.size() - size).length() < 2.0);
        // `polls` is a safety net in case the size is never reported.
        if sized || self.polls > 8 {
            self.shown = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            return true;
        }
        ctx.request_repaint();
        false
    }
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
    /// Why the last captured combo was refused (shown while recording).
    record_hint: Option<String>,
    admin_status: Option<String>,
    autostart_open: bool,
    autostart_sel: AutostartMethod,
    autostart_status: Option<String>,
    /// One-off note from the startup autostart check (stale path repaired).
    autostart_note: Option<String>,
    autostart_win: ChildWindow,
    advanced_open: bool,
    advanced_win: ChildWindow,
    /// The ⋯ menu is drawn in this window as a floating card under the
    /// button (see `menu_popup`).
    menu_open: bool,
    /// ⋯ button rect from the previous frame — the menu is anchored to it.
    menu_button_rect: egui::Rect,
    /// Menu card rect from the previous frame, for click-outside tests.
    menu_rect: egui::Rect,
    /// `InputState::time` when the menu was opened, drives the animation.
    menu_opened_at: f64,
    /// Last known focus state of our own window (for the debug log).
    was_focused: bool,
    /// Keys currently held, tracked by the in-window capture poll.
    rec_down: std::collections::HashSet<u32>,
    /// The diagnostics report has been copied at least once.
    diag_copied: bool,
}

impl App {
    fn new(
        shared: Arc<Shared>,
        cmds: Sender<WorkerCmd>,
        worker: JoinHandle<()>,
        icon: Arc<egui::IconData>,
    ) -> Self {
        // Verify the autostart entry at startup: the registry value, the
        // shortcut or the scheduled task outlives the exe it points at, so
        // after an update it can point at an old build or be an empty
        // command left by an earlier version. Repairing it silently would
        // hide the problem, so a note is kept for the Autostart window.
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let method = shared.config.lock().unwrap().autostart;
                if method == AutostartMethod::None {
                    return;
                }
                let state = config::autostart_state(method);
                crate::diag::log(&format!("autostart check: {state:?}"));
                if !state.is_broken() {
                    // A stale-but-valid path is reported in the Autostart
                    // window, not rewritten behind the user's back.
                    if state.is_stale_but_valid() {
                        if let Some(note) = state.message() {
                            *shared.autostart_note.lock().unwrap() = Some(note);
                        }
                    }
                    return;
                }
                let start_minimized = shared.config.lock().unwrap().start_minimized;
                // Recreating the logon task needs administrator rights; a
                // background repair cannot ask for them.
                let allowed = !method.requires_admin() || elevation::is_elevated();
                let repaired =
                    allowed && config::apply_autostart(method, start_minimized).is_ok();
                crate::diag::log(&format!("autostart repair: ok={repaired}"));
                if let Some(note) = state.message() {
                    let note = if repaired {
                        note
                    } else {
                        format!("{note} (repair failed — re-apply manually)")
                    };
                    *shared.autostart_note.lock().unwrap() = Some(note);
                }
            });
        }

        Self {
            shared,
            cmds,
            worker: Some(worker),
            icon,
            elevated: elevation::is_elevated(),
            recording: false,
            record_hint: None,
            admin_status: None,
            autostart_open: false,
            autostart_sel: AutostartMethod::RegistryRun,
            autostart_status: None,
            autostart_note: None,
            autostart_win: ChildWindow::default(),
            advanced_open: false,
            advanced_win: ChildWindow::default(),
            menu_open: false,
            menu_button_rect: egui::Rect::NOTHING,
            menu_rect: egui::Rect::NOTHING,
            menu_opened_at: 0.0,
            was_focused: false,
            rec_down: std::collections::HashSet::new(),
            diag_copied: false,
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

    fn set_autostart(&mut self, ctx: &egui::Context, method: AutostartMethod) {
        // Write first: the elevated helper applies whatever the config says.
        let start_minimized = {
            let mut cfg = self.shared.config.lock().unwrap();
            cfg.autostart = method;
            let _ = config::save(&cfg);
            cfg.start_minimized
        };
        self.autostart_status = None;
        self.apply_autostart(ctx, method, start_minimized);
    }

    fn set_start_minimized(&mut self, ctx: &egui::Context, on: bool) {
        let (method, start_minimized) = {
            let mut cfg = self.shared.config.lock().unwrap();
            cfg.start_minimized = on;
            let _ = config::save(&cfg);
            (cfg.autostart, cfg.start_minimized)
        };
        // Re-apply so the entry picks up the new argument list.
        if method != AutostartMethod::None {
            self.apply_autostart(ctx, method, start_minimized);
        }
    }

    /// Apply the autostart entry. The logon task needs administrator rights,
    /// so it is created by a short-lived elevated helper. The running
    /// instance is not restarted: it keeps its window, tray icon and hook.
    fn apply_autostart(
        &mut self,
        ctx: &egui::Context,
        method: AutostartMethod,
        start_minimized: bool,
    ) {
        if method.requires_admin() && !elevation::is_elevated() {
            let args = format!("--apply-autostart {}", method.key());
            let shared = self.shared.clone();
            let ctx = ctx.clone();
            self.autostart_status = None;
            shared.autostart_pending.store(true, Ordering::SeqCst);
            std::thread::spawn(move || {
                let ok = elevation::run_elevated_wait(&args);
                shared.autostart_pending.store(false, Ordering::SeqCst);
                crate::diag::log(&format!("autostart helper finished: ok={ok}"));
                *shared.autostart_note.lock().unwrap() = Some(if ok {
                    "Autostart updated.".to_string()
                } else {
                    "Autostart was not updated: administrator rights were not granted.".to_string()
                });
                ctx.request_repaint();
            });
            return;
        }
        match config::apply_autostart(method, start_minimized) {
            Ok(()) => {
                crate::diag::log(&format!("autostart applied: {}", method.key()));
                self.autostart_status = None;
                self.autostart_note = None;
            }
            Err(e) => {
                crate::diag::log(&format!("autostart failed: {e}"));
                self.autostart_status = Some(e.to_string());
            }
        }
    }

    fn set_caps_combo(&self, combo: CapsCombo) {
        let next = {
            let cfg = self.shared.config.lock().unwrap();
            let mut next = cfg.clone();
            next.caps_combo = combo;
            next
        };
        let _ = config::save(&next);
        *self.shared.config.lock().unwrap() = next;
    }

    /// Read the physical key state while recording and turn the first
    /// non-modifier press into a hotkey. `GetAsyncKeyState` is global, so
    /// this works regardless of which window the hook can see.
    fn poll_capture(&mut self) {
        let mods = crate::hook::current_modifiers();
        for &vk_code in record_key_list() {
            let down = crate::hook::key_down(vk_code);
            let was = self.rec_down.contains(&vk_code);
            if down && !was {
                self.rec_down.insert(vk_code);
                if vk_code == crate::vk::VK_ESCAPE {
                    self.recording = false;
                    self.record_hint = None;
                    self.shared.capturing.store(false, Ordering::SeqCst);
                    return;
                }
                if crate::vk::is_modifier(vk_code) {
                    continue;
                }
                let hotkey = Hotkey {
                    vk: vk_code,
                    ctrl: mods.ctrl,
                    alt: mods.alt,
                    shift: mods.shift,
                    win: mods.win,
                };
                match hotkey.validate() {
                    Ok(()) => {
                        self.recording = false;
                        self.record_hint = None;
                        self.shared.capturing.store(false, Ordering::SeqCst);
                        self.set_hotkey(hotkey);
                        return;
                    }
                    Err(hint) => {
                        self.record_hint = Some(hint.to_string());
                        return;
                    }
                }
            } else if !down && was {
                self.rec_down.remove(&vk_code);
            }
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

    /// Center a child window builder on the main window.
    fn child_builder(
        &self,
        ctx: &egui::Context,
        title: &str,
        size: [f32; 2],
    ) -> egui::ViewportBuilder {
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(size)
            .with_resizable(false)
            .with_icon(self.icon.clone());
        if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
            let c = outer.center();
            builder = builder
                .with_position(egui::pos2(c.x - size[0] / 2.0, c.y - size[1] / 2.0));
        }
        builder
    }

    fn advanced_viewport(&mut self, ctx: &egui::Context, cfg: &Config) {
        let size = egui::vec2(300.0, 168.0);
        let builder = self
            .child_builder(ctx, "Advanced settings", [size.x, size.y])
            .with_visible(self.advanced_win.shown);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("advanced-window"),
            builder,
            |ctx, _class| {
                if !self.advanced_win.ready(ctx, size) {
                    return;
                }
                if ctx.input(|i| i.viewport().close_requested()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.advanced_open = false;
                    self.advanced_win.reset();
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new("Caps Lock toggles with:").selectable(true));
                        egui::ComboBox::from_id_salt("caps-combo")
                            .selected_text(cfg.caps_combo.title())
                            .show_ui(ui, |ui| {
                                for c in CapsCombo::ALL {
                                    if ui.selectable_label(cfg.caps_combo == c, c.title()).clicked()
                                    {
                                        self.set_caps_combo(c);
                                    }
                                }
                            });
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let mut logging = crate::diag::is_enabled();
                    if ui
                        .checkbox(&mut logging, "Debug log (this session only)")
                        .changed()
                    {
                        crate::diag::enable(logging);
                    }
                    ui.add_space(4.0);
                    if ui.button("Copy diagnostics").clicked() {
                        ctx.copy_text(crate::diag::report(cfg));
                        self.diag_copied = true;
                    }
                    if self.diag_copied {
                        ui.add_space(2.0);
                        ui.label("Copied to clipboard.");
                    }
                });
            },
        );
    }

    fn autostart_viewport(&mut self, ctx: &egui::Context, cfg: &Config) {
        // Pop next to the main window instead of wherever the WM puts it.
        let size = egui::vec2(320.0, 216.0);
        let builder = self
            .child_builder(ctx, "Autostart", [size.x, size.y])
            .with_visible(self.autostart_win.shown);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("autostart-window"),
            builder,
            |ctx, _class| {
                if !self.autostart_win.ready(ctx, size) {
                    return;
                }
                if ctx.input(|i| i.viewport().close_requested()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.autostart_open = false;
                    self.autostart_win.reset();
                    // The note is a one-off answer to the last action; keep
                    // it for this visit only.
                    self.autostart_note = None;
                    self.autostart_status = None;
                    *self.shared.autostart_note.lock().unwrap() = None;
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
                            self.set_autostart(ctx, if on { sel } else { AutostartMethod::None });
                        }
                    });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Method:");
                        // Always enabled: pick the method first, then flip
                        // the toggle — or switch methods while already on.
                        egui::ComboBox::from_id_salt("autostart-method")
                            .selected_text(sel.title())
                            .show_ui(ui, |ui| {
                                for m in AutostartMethod::ALL {
                                    let label = if m.requires_admin() {
                                        format!("{} (recommended)", m.title())
                                    } else {
                                        m.title().to_string()
                                    };
                                    if ui.selectable_label(sel == m, label).clicked() {
                                        self.autostart_sel = m;
                                        if on {
                                            self.set_autostart(ctx, m);
                                        }
                                    }
                                }
                            });
                    });

                    ui.add_space(8.0);
                    let mut minimized = cfg.start_minimized;
                    if ui
                        .checkbox(&mut minimized, "Start minimized in the tray")
                        .changed()
                    {
                        self.set_start_minimized(ctx, minimized);
                    }

                    // Registry and Startup-folder entries run with the normal
                    // user token: only the logon task can start elevated, and
                    // only it does so without a UAC prompt.
                    if sel != AutostartMethod::None && !sel.requires_admin() {
                        ui.add_space(8.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(240, 180, 90),
                            "Starts without administrator rights. The scheduled task is the only \
                             method that starts elevated, and it does so without a UAC prompt.",
                        );
                    }

                    if self.shared.autostart_pending.load(Ordering::SeqCst) {
                        ui.add_space(6.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(150, 150, 150),
                            "Waiting for the UAC prompt…",
                        );
                    }
                    if let Some(note) = &self.autostart_note {
                        ui.add_space(6.0);
                        ui.colored_label(egui::Color32::from_rgb(240, 180, 90), note.as_str());
                    }
                    if let Some(err) = &self.autostart_status {
                        ui.add_space(6.0);
                        ui.colored_label(egui::Color32::LIGHT_RED, err.as_str());
                    }
                });
            },
        );
    }

    /// The ⋯ menu: a card under the button, drawn inside this window.
    ///
    /// `popup_below_widget` (the first attempt) sizes a popup to the width
    /// of the widget it hangs off; the "…" button is ~24 pt, so every label
    /// was squeezed into a column of single letters. The width is now
    /// measured from the labels themselves, so nothing wraps. The card fades
    /// in (egui's own `Area` fade) and slides up into place — no extra OS
    /// window, so nothing flashes or jumps on the way in.
    fn menu_popup(&mut self, ctx: &egui::Context) {
        let font = menu_font();
        let text_w = ctx.fonts(|f| {
            MENU_ITEMS
                .iter()
                .map(|(_, label)| {
                    f.layout_no_wrap((*label).to_owned(), font.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
                .fold(0.0_f32, f32::max)
        });
        let content_w = (MENU_TEXT_LEFT + text_w + 14.0).max(112.0).round();

        let frame = egui::Frame::popup(&ctx.style());
        let pad = frame.total_margin().sum().x;

        // Slide: the card starts a few points lower and eases up under the
        // button, in step with the fade `Area` applies on its first frame.
        let anim = ctx.style().animation_time as f64;
        let t = (((ctx.input(|i| i.time) - self.menu_opened_at) / anim.max(0.001)) as f32)
            .clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint();
        }
        let ease = 1.0 - (1.0 - t).powi(3);
        let anchor = self.menu_button_rect;
        let pos = egui::pos2(
            anchor.right() - (content_w + pad),
            anchor.bottom() + 2.0 + (1.0 - ease) * MENU_SLIDE,
        );

        let mut chosen = None;
        let area = egui::Area::new(egui::Id::new("rasklad-main-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .movable(false)
            .interactable(true);

        let shown = area.show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(content_w);
                ui.spacing_mut().item_spacing.y = 0.0;
                let text_color = ui.visuals().text_color();
                let hot_color = ui.visuals().strong_text_color();
                let hover_fill = ui.visuals().widgets.hovered.weak_bg_fill;
                let sep_color = ui.visuals().window_stroke().color.gamma_multiply(0.7);

                for (action, label) in MENU_ITEMS {
                    if action == MenuAction::Exit {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(content_w, MENU_SEP_H),
                            egui::Sense::hover(),
                        );
                        let y = rect.center().y;
                        ui.painter().line_segment(
                            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                            egui::Stroke::new(1.0_f32, sep_color),
                        );
                    }

                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(content_w, MENU_ITEM_H),
                        egui::Sense::click(),
                    );
                    let hot = resp.hovered();
                    if hot {
                        ui.painter().rect_filled(rect, 4.0, hover_fill);
                    }
                    ui.painter().text(
                        egui::pos2(rect.left() + MENU_TEXT_LEFT, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        font.clone(),
                        if hot { hot_color } else { text_color },
                    );
                    if resp.clicked() {
                        chosen = Some(action);
                    }
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            });
        });

        self.menu_rect = shown.response.rect;

        if let Some(action) = chosen {
            self.menu_open = false;
            match action {
                MenuAction::Autostart => self.autostart_open = true,
                MenuAction::Advanced => self.advanced_open = true,
                MenuAction::Exit => self.quit(),
            }
        }
    }

    /// Esc, a click outside the button/menu, or the window losing focus
    /// closes the menu.
    fn dismiss_menu(&mut self, ctx: &egui::Context) {
        if !self.menu_open {
            return;
        }
        let inside = self.menu_button_rect.union(self.menu_rect).expand(4.0);
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx
                .input(|i| i.pointer.interact_pos())
                .map_or(true, |p| !inside.contains(p));
        let lost_focus = ctx.input(|i| i.viewport().focused) == Some(false);
        if clicked_outside || lost_focus || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.menu_open = false;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Hotkey capture finished in the hook.
        if let Some(res) = self.shared.capture.lock().unwrap().take() {
            match res {
                CaptureResult::Captured(hk) => {
                    self.recording = false;
                    self.record_hint = None;
                    self.set_hotkey(hk);
                }
                CaptureResult::Cancelled => {
                    self.recording = false;
                    self.record_hint = None;
                }
                // Not a usable hotkey: keep recording, just explain why.
                CaptureResult::Rejected(hint) => {
                    self.record_hint = Some(hint.to_string());
                }
            }
        }

        if self.autostart_note.is_none() {
            if let Some(note) = self.shared.autostart_note.lock().unwrap().take() {
                self.autostart_note = Some(note);
            }
        }

        // Window close button hides to tray instead of exiting.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.menu_open = false;
        }

        self.dismiss_menu(ctx);

        // Hotkey capture also runs here, not only in the hook: the global
        // hook is not called for keystrokes aimed at our own window on this
        // machine, and recording is always done in this window.
        if self.recording {
            ctx.request_repaint_after(Duration::from_millis(16));
            self.poll_capture();
        }

        // Log focus transitions: the hook not seeing keys while our own
        // window is focused is the one thing that needs a trace.
        let focused = ctx.input(|i| i.viewport().focused) == Some(true);
        if focused != self.was_focused {
            self.was_focused = focused;
            crate::diag::log(&format!("window focused={focused}"));
        }

        let cfg = self.config();

        egui::CentralPanel::default().show(ctx, |ui| {
            // No in-window title — the title bar already says "Rasklad".
            // The ⋯ button shares the subtitle row so no dead strip is left.
            ui.horizontal(|ui| {
                ui.add(egui::Label::new("Switch keyboard layout with a custom hotkey.").selectable(true));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The menu is drawn by `menu_popup` — a measured width,
                    // so labels never wrap, and anchored to this button.
                    let resp = ui.button("…");
                    self.menu_button_rect = resp.rect;
                    if resp.clicked() {
                        if self.menu_open {
                            self.menu_open = false;
                        } else {
                            self.menu_open = true;
                            self.menu_opened_at = ctx.input(|i| i.time);
                        }
                    }
                });
            });
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
                        self.record_hint = None;
                        self.shared.capturing.store(false, Ordering::SeqCst);
                    }
                } else {
                    if ui.button("Record").clicked() {
                        self.recording = true;
                        self.record_hint = None;
                        // Seed with keys already held so a modifier that was
                        // down before recording is not treated as a press.
                        self.rec_down = record_key_list()
                            .iter()
                            .copied()
                            .filter(|&vk| crate::hook::key_down(vk))
                            .collect();
                        self.shared.capturing.store(true, Ordering::SeqCst);
                    }
                    if ui.button("Reset").clicked() {
                        self.set_hotkey(Hotkey::default());
                    }
                }
            });
            if self.recording {
                if let Some(hint) = &self.record_hint {
                    ui.add_space(2.0);
                    ui.colored_label(egui::Color32::from_rgb(240, 180, 90), hint.as_str());
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(4.0);
            ui.add(egui::Label::new(
                "Lives in the system tray.",
            ).selectable(true));
            if !self.elevated {
                ui.add(egui::Label::new(
                    "Not running as administrator — layout switching",
                ).selectable(true));
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new("won't work in elevated windows.").selectable(true));
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
        if self.advanced_open {
            self.advanced_viewport(ctx, &cfg);
        }
        if self.menu_open {
            self.menu_popup(ctx);
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
    // Window/taskbar icon: 64px stays sharp on scaled displays and
    // Windows derives the 16px title-bar icon from it.
    let (rgba, w, h) = assets::icon_rgba(64);
    let icon = Arc::new(egui::IconData {
        rgba,
        width: w,
        height: h,
    });

    // The admin warning takes ~60px at the bottom — don't reserve it
    // when running elevated.
    let height = if elevation::is_elevated() { 140.0 } else { 200.0 };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rasklad")
            .with_inner_size([340.0, height])
            .with_resizable(false)
            .with_visible(visible)
            .with_icon(icon.clone()),
        ..Default::default()
    };

    let app_cmds = cmds.clone();
    let _ = eframe::run_native(
        "Rasklad",
        options,
        Box::new(move |cc| {
            // Only explanatory text is selectable; button/field labels aren't.
            cc.egui_ctx.style_mut(|s| s.interaction.selectable_labels = false);
            spawn_events(cc.egui_ctx.clone(), shared.clone(), app_cmds, show_event.0 as usize);
            Ok(Box::new(App::new(shared, cmds, worker, icon)))
        }),
    );
}

/// Virtual keys polled while recording a hotkey. Covers letters, digits,
/// function keys, navigation/editing keys, numpad, punctuation, media and
/// the modifiers themselves (tracked, never used as the main key).
fn record_key_list() -> &'static [u32] {
    use std::sync::OnceLock;
    static KEYS: OnceLock<Vec<u32>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut v = vec![0x08, 0x09, 0x0D, 0x13, 0x14, 0x1B, 0x20, 0x2C, 0x2D, 0x2E, 0x90, 0x91];
        v.extend(0x21..=0x28); // PgUp..Down, arrows
        v.extend(0x30..=0x39); // digits
        v.extend(0x41..=0x5A); // letters
        v.extend(0x5B..=0x5D); // Win, Win, Menu
        v.extend(0x60..=0x6F); // numpad
        v.extend(0x70..=0x87); // F1..F24
        v.extend(0xA0..=0xA5); // L/R shift, ctrl, alt
        v.extend(0xBA..=0xC0); // ; = , - . / ~
        v.extend(0xDB..=0xDE); // [ \\ ] '
        v
    })
}
