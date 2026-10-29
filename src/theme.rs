//! Window colours: the Windows light or dark theme, plus its accent colour.
//!
//! The accent is the one from Settings -> Personalisation -> Colours: the
//! colour of the Start menu, the taskbar badges and the selection highlight.
//! Windows keeps it in the registry as `0xAABBGGRR`, so the byte order is not
//! the one most people expect.

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use eframe::egui::style::WidgetVisuals;
use eframe::egui::{Color32, Stroke, Visuals};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, WM_SETTINGCHANGE, WNDCLASSW, WS_POPUP,
};

use crate::shared::Shared;

/// How much the accent has to stand out from the panel to be used at all.
/// Below this it would be a fill nobody can see, so the neutral palette wins.
const MIN_ACCENT_CONTRAST: f32 = 1.25;
/// How much the accent has to stand out when it is used as text.
const MIN_TEXT_CONTRAST: f32 = 3.0;

/// Read the current palette from Windows.
pub fn current() -> Visuals {
    build(crate::winshow::system_uses_light_theme(), accent_rgb())
}

/// Build a palette for the given mode and accent.
///
/// Pure on purpose: the rules here are worth testing without a desktop to
/// change the theme on.
pub fn build(light: bool, accent: Option<(u8, u8, u8)>) -> Visuals {
    let mut visuals = if light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    let panel = visuals.panel_fill;
    let accent = accent
        .map(|(r, g, b)| Color32::from_rgb(r, g, b))
        .filter(|colour| contrast_ratio(*colour, panel) >= MIN_ACCENT_CONTRAST);

    if let Some(accent) = accent {
        let on_accent = readable_on(accent);
        // Text on the panel needs more contrast than a fill does, so a pale
        // accent is darkened until it reads as a link.
        let as_text = make_readable(accent, panel, MIN_TEXT_CONTRAST);

        visuals.selection.bg_fill = accent;
        visuals.selection.stroke = Stroke::new(1.0_f32, on_accent);
        visuals.hyperlink_color = as_text;
        visuals.widgets.active.bg_fill = accent;
        visuals.widgets.active.weak_bg_fill = accent;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, on_accent);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent);
    }

    visuals
}

static WATCHED: OnceLock<Arc<Shared>> = OnceLock::new();

/// Receives the broadcast Windows sends when colours change.
///
/// A message-only window would not do: broadcasts go to top-level windows
/// only, and a message-only window is not one. This one is top-level and
/// never shown.
unsafe extern "system" fn watcher_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_SETTINGCHANGE {
        if let Some(shared) = WATCHED.get() {
            shared.theme_dirty.store(true, Ordering::SeqCst);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Start watching for theme and accent changes.
///
/// Must be called from a thread that pumps messages, which the worker already
/// does for the tray icon and the hook.
pub fn install_watcher(shared: Arc<Shared>) {
    let _ = WATCHED.set(shared);
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            crate::diag::log("theme watcher: no module handle");
            return;
        };
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(watcher_proc),
            hInstance: instance,
            lpszClassName: w!("rasklad-settings-watch"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            crate::diag::log("theme watcher: could not register the class");
            return;
        }
        let hwnd = CreateWindowExW(
            Default::default(),
            w!("rasklad-settings-watch"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        );
        match hwnd {
            Ok(_) => crate::diag::log("theme watcher installed"),
            Err(e) => crate::diag::log(&format!("theme watcher: {e:?}")),
        }
    }
}

/// How long a theme change takes to fade, in seconds.
pub const FADE_SECONDS: f64 = 0.18;

/// Blend two palettes. `t` runs from 0 (the old one) to 1 (the new one).
///
/// Only colours are blended; shapes, shadows and flags come from the new
/// palette, since animating a corner radius or a shadow offset is not what a
/// theme change is about.
pub fn lerp(from: &Visuals, to: &Visuals, t: f32) -> Visuals {
    let mut out = to.clone();
    out.override_text_color = match (from.override_text_color, to.override_text_color) {
        (Some(a), Some(b)) => Some(mix(a, b, t)),
        _ => to.override_text_color,
    };
    out.widgets.noninteractive =
        lerp_widget(&from.widgets.noninteractive, &to.widgets.noninteractive, t);
    out.widgets.inactive = lerp_widget(&from.widgets.inactive, &to.widgets.inactive, t);
    out.widgets.hovered = lerp_widget(&from.widgets.hovered, &to.widgets.hovered, t);
    out.widgets.active = lerp_widget(&from.widgets.active, &to.widgets.active, t);
    out.widgets.open = lerp_widget(&from.widgets.open, &to.widgets.open, t);
    out.selection.bg_fill = mix(from.selection.bg_fill, to.selection.bg_fill, t);
    out.selection.stroke.color = mix(from.selection.stroke.color, to.selection.stroke.color, t);
    out.hyperlink_color = mix(from.hyperlink_color, to.hyperlink_color, t);
    out.faint_bg_color = mix(from.faint_bg_color, to.faint_bg_color, t);
    out.extreme_bg_color = mix(from.extreme_bg_color, to.extreme_bg_color, t);
    out.code_bg_color = mix(from.code_bg_color, to.code_bg_color, t);
    out.warn_fg_color = mix(from.warn_fg_color, to.warn_fg_color, t);
    out.error_fg_color = mix(from.error_fg_color, to.error_fg_color, t);
    out.window_fill = mix(from.window_fill, to.window_fill, t);
    out.window_stroke.color = mix(from.window_stroke.color, to.window_stroke.color, t);
    out.panel_fill = mix(from.panel_fill, to.panel_fill, t);
    out.text_cursor.stroke.color = mix(
        from.text_cursor.stroke.color,
        to.text_cursor.stroke.color,
        t,
    );
    out
}

fn lerp_widget(from: &WidgetVisuals, to: &WidgetVisuals, t: f32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: mix(from.bg_fill, to.bg_fill, t),
        weak_bg_fill: mix(from.weak_bg_fill, to.weak_bg_fill, t),
        bg_stroke: Stroke::new(
            to.bg_stroke.width,
            mix(from.bg_stroke.color, to.bg_stroke.color, t),
        ),
        fg_stroke: Stroke::new(
            to.fg_stroke.width,
            mix(from.fg_stroke.color, to.fg_stroke.color, t),
        ),
        ..*to
    }
}

/// Ease in and out, so the fade does not start and stop abruptly.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Windows stores the accent as `0xAABBGGRR`: red in the low byte.
pub fn from_abgr(value: u32) -> (u8, u8, u8) {
    (
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
    )
}

fn accent_rgb() -> Option<(u8, u8, u8)> {
    use winreg::enums::HKEY_CURRENT_USER;

    let key = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\DWM")
        .ok()?;
    let value: u32 = key.get_value("AccentColor").ok()?;
    Some(from_abgr(value))
}

/// WCAG relative luminance.
fn luminance(colour: Color32) -> f32 {
    let channel = |value: u8| {
        let v = value as f32 / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
}

/// WCAG contrast ratio, from 1.0 (identical) to 21.0 (black on white).
fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (high, low) = if la > lb { (la, lb) } else { (lb, la) };
    (high + 0.05) / (low + 0.05)
}

/// Black or white, whichever reads better on `background`.
fn readable_on(background: Color32) -> Color32 {
    if contrast_ratio(background, Color32::BLACK) >= contrast_ratio(background, Color32::WHITE) {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Move `colour` towards black or white until it reads on `panel`.
fn make_readable(colour: Color32, panel: Color32, wanted: f32) -> Color32 {
    if contrast_ratio(colour, panel) >= wanted {
        return colour;
    }
    let target = if luminance(panel) > 0.5 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let mut best = colour;
    for step in 1..=10 {
        let mixed = mix(colour, target, step as f32 / 10.0);
        if contrast_ratio(mixed, panel) >= wanted {
            return mixed;
        }
        best = mixed;
    }
    best
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let blend = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(
        blend(a.r(), b.r()),
        blend(a.g(), b.g()),
        blend(a.b(), b.b()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The accent of the machine this was written on: a muted green.
    const GREEN: (u8, u8, u8) = (0x48, 0x68, 0x60);

    #[test]
    fn the_accent_is_stored_backwards() {
        // 0xAABBGGRR, red in the low byte.
        assert_eq!(from_abgr(0xFF60_6848), GREEN);
        assert_eq!(from_abgr(0xFF00_0000), (0, 0, 0));
        assert_eq!(from_abgr(0xFF00_00FF), (255, 0, 0));
    }

    #[test]
    fn text_on_the_accent_is_picked_for_contrast() {
        assert_eq!(
            readable_on(Color32::from_rgb(250, 230, 120)),
            Color32::BLACK
        );
        assert_eq!(readable_on(Color32::from_rgb(20, 30, 90)), Color32::WHITE);
    }

    #[test]
    fn an_accent_the_same_colour_as_the_panel_is_ignored() {
        let panel = Visuals::dark().panel_fill;
        let near = (
            panel.r().saturating_add(6),
            panel.g().saturating_add(6),
            panel.b().saturating_add(6),
        );
        let visuals = build(false, Some(near));
        // The neutral palette is left alone.
        assert_eq!(visuals.selection.bg_fill, Visuals::dark().selection.bg_fill);
    }

    #[test]
    fn a_usable_accent_colours_the_selection() {
        let visuals = build(false, Some(GREEN));
        let accent = Color32::from_rgb(GREEN.0, GREEN.1, GREEN.2);
        assert_eq!(visuals.selection.bg_fill, accent);
        assert_eq!(visuals.widgets.active.bg_fill, accent);
        // Text drawn on the accent is chosen, not inherited from the theme.
        let on_accent = visuals.selection.stroke.color;
        assert!(on_accent == Color32::BLACK || on_accent == Color32::WHITE);
        assert!(contrast_ratio(on_accent, accent) >= 4.5);
    }

    #[test]
    fn a_pale_accent_is_darkened_before_it_is_used_as_text() {
        // A light blue that reads as a fill but not as a link on white.
        let pale = (120, 180, 240);
        let visuals = build(true, Some(pale));
        let accent = Color32::from_rgb(pale.0, pale.1, pale.2);
        let panel = visuals.panel_fill;
        assert_eq!(visuals.selection.bg_fill, accent);
        assert!(contrast_ratio(visuals.hyperlink_color, panel) >= MIN_TEXT_CONTRAST);
        // Darkened, not replaced: it still has to look like the accent.
        assert!(luminance(visuals.hyperlink_color) < luminance(accent));
    }

    #[test]
    fn blending_ends_on_the_target_palette() {
        let dark = Visuals::dark();
        let light = Visuals::light();
        let start = lerp(&dark, &light, 0.0);
        let end = lerp(&dark, &light, 1.0);
        assert_eq!(start.panel_fill, dark.panel_fill);
        assert_eq!(end.panel_fill, light.panel_fill);
        assert_eq!(end.widgets.inactive.bg_fill, light.widgets.inactive.bg_fill);
        assert_eq!(end.selection.bg_fill, light.selection.bg_fill);
        assert_eq!(end.window_stroke.color, light.window_stroke.color);
    }

    #[test]
    fn blending_passes_through_the_middle() {
        let dark = Visuals::dark();
        let light = Visuals::light();
        let mid = lerp(&dark, &light, 0.5);
        assert_ne!(mid.panel_fill, dark.panel_fill);
        assert_ne!(mid.panel_fill, light.panel_fill);
        assert!(luminance(mid.panel_fill) > luminance(dark.panel_fill));
        assert!(luminance(mid.panel_fill) < luminance(light.panel_fill));
    }

    #[test]
    fn easing_starts_and_ends_flat() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert!((ease(0.5) - 0.5).abs() < 1e-6);
        // Slower than linear at the start, faster towards the end.
        assert!(ease(0.25) < 0.25);
        assert!(ease(0.75) > 0.75);
        // Out of range input is clamped, not extrapolated.
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
    }

    #[test]
    fn without_an_accent_nothing_changes() {
        let plain = build(true, None);
        assert_eq!(plain.selection.bg_fill, Visuals::light().selection.bg_fill);
    }
}
