//! Native helpers to pop the main egui window to the foreground.
//!
//! `egui::Context::send_viewport_cmd` alone is not enough here: a hidden
//! window re-shown by a background process is refused foreground focus by
//! Windows and only flashes on the taskbar. `AttachThreadInput` to the
//! foreground thread is the classic workaround.

#![cfg(windows)]

use windows::core::BOOL;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

const TITLE: &[u16] = &[b'R' as u16, b'a' as u16, b's' as u16, b'k' as u16, b'l' as u16, b'a' as u16, b'd' as u16, 0];

struct EnumCtx {
    pid: u32,
    found: HWND,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> BOOL {
    let ctx = &mut *(lp.0 as *mut EnumCtx);
    let mut wpid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut wpid));
    if wpid == ctx.pid {
        let len = GetWindowTextLengthW(hwnd);
        if len > 0 {
            let mut buf = [0u16; 64];
            let n = GetWindowTextW(hwnd, &mut buf) as usize;
            if n == TITLE.len() - 1 && buf[..n] == TITLE[..TITLE.len() - 1] {
                ctx.found = hwnd;
                return BOOL(0); // stop enumeration
            }
        }
    }
    BOOL(1)
}

/// Top-level window of this process titled "rasklad", if it exists.
fn find_main_window() -> Option<HWND> {
    unsafe {
        let mut ctx = EnumCtx {
            pid: GetCurrentProcessId(),
            found: HWND(std::ptr::null_mut()),
        };
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut _ as isize));
        if ctx.found.0.is_null() {
            None
        } else {
            Some(ctx.found)
        }
    }
}

/// Make the main window fully transparent.
///
/// eframe paints one frame before we get a chance to hide the window, and
/// that frame is a white rectangle. Blending it away with a zero alpha means
/// a `--minimized` start shows nothing at all, not even a flicker.
pub fn fade_out() -> bool {
    let Some(hwnd) = find_main_window() else {
        return false;
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            style | WS_EX_LAYERED.0 as isize,
        );
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA).is_ok()
    }
}

/// Undo `fade_out`. Called before the window is shown for real.
pub fn fade_in() {
    let Some(hwnd) = find_main_window() else {
        return;
    };
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
    }
}

/// Hide the main window if it is currently on screen. Returns true when it
/// was visible and has now been hidden.
///
/// Used for a `--minimized` start: eframe shows the window after its first
/// frame no matter what the viewport builder says, and a hidden window gets
/// no further frames, so the viewport command alone cannot undo it.
pub fn hide_main_window_if_visible() -> bool {
    let Some(hwnd) = find_main_window() else {
        return false;
    };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
        true
    }
}

/// Restore + show the main window and force it into the foreground.
pub fn raise_main_window() {
    let Some(hwnd) = find_main_window() else { return };
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let my_tid = GetCurrentThreadId();
        let attached = fg_tid != 0
            && fg_tid != my_tid
            && AttachThreadInput(my_tid, fg_tid, true).as_bool();

        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));

        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
    }
}
