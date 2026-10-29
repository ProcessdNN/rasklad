//! Embedded app icon, decoded at the size the caller needs.
//!
//! The `.ico` carries a hand-tuned frame for every size Windows asks for
//! (16, 20, 24, 32, 40, 48, 64, 128, 256), so the tray and the title bar
//! get the artist's pixels instead of a downscaled 256px bitmap.
//!
//! Frames come in two flavours and both must be supported: uncompressed
//! DIB (what most icon editors export) and PNG (what newer ones do).
//! Supporting only PNG broke the build once already, when the asset was
//! re-exported.

const ICO: &[u8] = include_bytes!("../assets/icon.ico");

/// (rgba, width, height) of an icon exactly `size` pixels square.
/// A greyed-out copy of an RGBA image, used for the tray icon while the
/// hotkey is switched off.
///
/// Colour is pulled towards its own luminance and the whole image is made
/// half transparent, so it reads as "not active" on a light and on a dark
/// taskbar alike while the shape stays recognisable.
pub fn dimmed(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        let luma = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
        for channel in px.iter_mut().take(3) {
            *channel = (*channel as f32 * 0.4 + luma * 0.6) as u8;
        }
        px[3] = (px[3] as f32 * 0.55) as u8;
    }
    out
}

pub fn icon_rgba(size: u32) -> (Vec<u8>, u32, u32) {
    let frame = best_frame(ICO, size).expect("embedded icon frame");
    let (rgba, width, height) = frame_rgba(frame).expect("embedded icon frame data");
    if width == size && height == size {
        return (rgba, width, height);
    }
    // Only reached for sizes the .ico does not carry: scale the next frame
    // up, not the 256px one.
    let img = image::RgbaImage::from_raw(width, height, rgba).expect("icon frame buffer");
    let scaled = image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3);
    (scaled.into_raw(), size, size)
}

/// Smallest frame at least `size` pixels wide; the largest frame if none is.
fn best_frame(ico: &[u8], size: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
    let mut smallest_fit: Option<(u32, &[u8])> = None;
    let mut largest: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let entry = 6 + i * 16;
        // A width of 0 in the directory entry means 256.
        let width = match *ico.get(entry)? {
            0 => 256,
            v => v as u32,
        };
        let len = u32::from_le_bytes(ico.get(entry + 8..entry + 12)?.try_into().ok()?) as usize;
        let off = u32::from_le_bytes(ico.get(entry + 12..entry + 16)?.try_into().ok()?) as usize;
        let data = ico.get(off..off + len)?;
        if largest.is_none_or(|(w, _)| width > w) {
            largest = Some((width, data));
        }
        if width >= size && smallest_fit.is_none_or(|(w, _)| width < w) {
            smallest_fit = Some((width, data));
        }
    }
    smallest_fit.or(largest).map(|(_, data)| data)
}

/// Decode one icon frame, PNG or uncompressed DIB, into RGBA.
fn frame_rgba(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        let (w, h) = img.dimensions();
        return Some((img.into_raw(), w, h));
    }

    // BITMAPINFOHEADER: width, then twice the height (colour + AND mask),
    // 32bpp bottom-up BGRA rows. 8/4/1bpp palette frames are not produced
    // by current icon editors and are not supported.
    let header = data.get(0..40)?;
    let header_size = u32::from_le_bytes(header[0..4].try_into().ok()?) as usize;
    let width = i32::from_le_bytes(header[4..8].try_into().ok()?);
    let double_height = i32::from_le_bytes(header[8..12].try_into().ok()?);
    let bpp = u16::from_le_bytes(header[14..16].try_into().ok()?);
    let compression = u32::from_le_bytes(header[16..20].try_into().ok()?);
    if width <= 0 || double_height <= 0 || bpp != 32 || compression != 0 {
        return None;
    }
    let width = width as usize;
    let height = double_height as usize / 2;
    let stride = width * 4;
    let pixels = data.get(header_size..header_size + stride * height)?;

    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in (0..height).rev() {
        let row = &pixels[y * stride..][..stride];
        for px in row.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    Some((rgba, width as u32, height as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every frame the asset ships must decode, and none may come out
    /// blank - that is exactly how the PNG-only decoder failed.
    #[test]
    fn decodes_every_frame() {
        for size in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
            let (rgba, w, h) = icon_rgba(size);
            assert_eq!((w, h), (size, size), "frame {size} has the wrong size");
            assert_eq!(
                rgba.len(),
                (size * size * 4) as usize,
                "frame {size} buffer"
            );
            assert!(
                rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
                "frame {size} is fully transparent"
            );
        }
    }

    /// Sizes the .ico does not carry are scaled, not rejected.
    #[test]
    fn dimming_softens_the_colour_and_keeps_the_shape() {
        let rgba = vec![200u8, 100, 50, 255, 0, 0, 0, 0];
        let dim = dimmed(&rgba);
        assert_eq!(dim.len(), rgba.len());
        // Still visible, but softer.
        assert!(dim[3] > 0 && dim[3] < 255);
        // Colour pulled towards its own grey.
        assert!(dim[0] < 200 && dim[1] > 100);
        // A transparent pixel has nothing to soften.
        assert_eq!(dim[7], 0);
    }

    #[test]
    fn scales_missing_sizes() {
        let (rgba, w, h) = icon_rgba(96);
        assert_eq!((w, h), (96, 96));
        assert_eq!(rgba.len(), (96 * 96 * 4) as usize);
    }
}
