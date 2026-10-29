//! Single-instance guard plus a named event that a second instance uses to
//! ask the running one to show its window.

#![cfg(windows)]

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;

pub enum Instance {
    Primary {
        _mutex: HANDLE,
        show: HANDLE,
        quit: HANDLE,
    },
    Secondary,
}

/// Returns `Primary` for the first instance; for later instances it sets the
/// "show" event on the running one and returns `Secondary`.
pub fn acquire() -> Instance {
    unsafe {
        let mutex = match CreateMutexW(None, true, w!("rasklad-instance")) {
            Ok(m) => m,
            Err(_) => return Instance::Secondary,
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("rasklad-show")) {
                let _ = SetEvent(ev);
                let _ = CloseHandle(ev);
            }
            let _ = CloseHandle(mutex);
            return Instance::Secondary;
        }
        let show = CreateEventW(None, true, false, w!("rasklad-show")).unwrap_or_default();
        // `--reset` asks the running copy to leave through this one.
        let quit = CreateEventW(None, true, false, w!("rasklad-quit")).unwrap_or_default();
        Instance::Primary {
            _mutex: mutex,
            show,
            quit,
        }
    }
}

/// Ask the running instance to shut down. No-op when nothing is running.
pub fn request_quit() {
    unsafe {
        if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, w!("rasklad-quit")) {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
    }
}

/// Wait until no other instance holds the guard.
///
/// `--reset` needs this: deleting the config while another copy runs is
/// pointless, because that copy holds the settings in memory and writes them
/// back at the next change.
pub fn wait_until_free(timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        unsafe {
            match CreateMutexW(None, true, w!("rasklad-instance")) {
                Ok(guard) => {
                    let taken = GetLastError() == ERROR_ALREADY_EXISTS;
                    let _ = CloseHandle(guard);
                    if !taken {
                        return true;
                    }
                }
                Err(_) => return true,
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// For the elevated relaunch: the exiting low-privilege process may still
/// hold the mutex for a moment, so retry until we can become primary
/// instead of giving up and showing the old window.
pub fn acquire_primary_wait(max_ms: u64) -> Instance {
    let start = std::time::Instant::now();
    loop {
        match acquire() {
            primary @ Instance::Primary { .. } => return primary,
            Instance::Secondary => {
                if start.elapsed().as_millis() as u64 >= max_ms {
                    return Instance::Secondary;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}
