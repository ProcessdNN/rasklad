//! Elevation check and a UAC ("runas") self-relaunch.
//!
//! SendInput can inject the layout-switch keys into elevated windows, but
//! only when we run with the same privileges: as a normal user the input
//! is dropped by UIPI for elevated targets, so the GUI warns and offers a
//! one-click elevated relaunch.

#![cfg(windows)]

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Run this exe elevated with `args` and wait for it to finish.
///
/// Used for one-shot privileged work (registering the logon task) so the
/// running instance keeps its window, its tray icon and its hook. Returns
/// true when the helper was started and exited successfully; a cancelled
/// UAC prompt shows up as a false.
pub fn run_elevated_wait(args: &str) -> bool {
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let verb = windows::core::HSTRING::from("runas");
    let file = windows::core::HSTRING::from(exe.to_string_lossy().as_ref());
    let params = windows::core::HSTRING::from(args);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe {
        if ShellExecuteExW(&mut info).is_err() {
            return false;
        }
        if info.hProcess.is_invalid() {
            return false;
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        code == 0
    }
}

/// Ask the shell to start a second, elevated copy. Returns true when the
/// launch was accepted; the caller should then exit so the single-instance
/// guard hands control to the new process.
pub fn relaunch_elevated() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let exe_w: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        (ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(exe_w.as_ptr()),
            w!("--elevated"),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
        .0 as usize)
            > 32
    }
}
