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

const TITLE: &[u16] = &[
    b'R' as u16,
    b'a' as u16,
    b's' as u16,
    b'k' as u16,
    b'l' as u16,
    b'a' as u16,
    b'd' as u16,
    0,
];

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

/// Move the window to the middle of the primary screen.
pub fn center_main_window() {
    let Some(hwnd) = find_main_window() else {
        return;
    };
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Top-left corner of the main window, for the startup log.
pub fn main_window_position() -> Option<(i32, i32)> {
    let hwnd = find_main_window()?;
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        Some((rect.left, rect.top))
    }
}

/// Bring the window back on screen if it is parked outside it.
pub fn ensure_on_screen() {
    let Some(hwnd) = find_main_window() else {
        return;
    };
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        if rect.left < -10_000 || rect.top < -10_000 {
            center_main_window();
        }
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

/// Open a folder in Explorer. Returns false when the shell refused.
pub fn open_folder(path: &std::path::Path) -> bool {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let target = HSTRING::from(path.to_string_lossy().as_ref());
    unsafe {
        (ShellExecuteW(
            None,
            w!("open"),
            &target,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
        .0 as usize)
            > 32
    }
}

/// Restore + show the main window and force it into the foreground.
pub fn raise_main_window() {
    let Some(hwnd) = find_main_window() else {
        return;
    };
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let my_tid = GetCurrentThreadId();
        let attached =
            fg_tid != 0 && fg_tid != my_tid && AttachThreadInput(my_tid, fg_tid, true).as_bool();

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
