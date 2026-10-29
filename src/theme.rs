//! Window colours: the Windows light or dark theme, plus its accent colour.
//!
//! The accent is the one from Settings -> Personalisation -> Colours: the
//! colour of the Start menu, the taskbar badges and the selection highlight.
//! Windows keeps it in the registry as `0xAABBGGRR`, so the byte order is not
//! the one most people expect.

use eframe::egui::{Color32, Stroke, Visuals};

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
    fn without_an_accent_nothing_changes() {
        let plain = build(true, None);
        assert_eq!(plain.selection.bg_fill, Visuals::light().selection.bg_fill);
    }
}
