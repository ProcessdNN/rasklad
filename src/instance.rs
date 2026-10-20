//! Single-instance guard plus a named event that a second instance uses to
//! ask the running one to show its window.

#![cfg(windows)]

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;

pub enum Instance {
    Primary { _mutex: HANDLE, show: HANDLE },
    Secondary,
}

/// Returns `Primary` for the first instance; for later instances it sets the
/// "show" event on the running one and returns `Secondary`.
pub fn acquire() -> Instance {
    unsafe {
        let mutex = match CreateMutexW(None, true, w!("capswitch-instance")) {
            Ok(m) => m,
            Err(_) => return Instance::Secondary,
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("capswitch-show")) {
                let _ = SetEvent(ev);
                let _ = CloseHandle(ev);
            }
            let _ = CloseHandle(mutex);
            return Instance::Secondary;
        }
        let show = CreateEventW(None, true, false, w!("capswitch-show")).unwrap_or_default();
        Instance::Primary {
            _mutex: mutex,
            show,
        }
    }
}
