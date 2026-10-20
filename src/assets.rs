//! Embedded app icon, decoded once and shared by the tray and the window.

const PNG: &[u8] = include_bytes!("../assets/icon.png");

/// (rgba, width, height)
pub fn icon_rgba() -> (Vec<u8>, u32, u32) {
    let img = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
        .expect("embedded icon png");
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    (rgba.into_raw(), w, h)
}
