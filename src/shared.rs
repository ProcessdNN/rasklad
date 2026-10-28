use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Mutex;

use crate::config::Config;
use crate::hotkey::Hotkey;

/// Result of a hotkey-capture session in the keyboard hook.
#[derive(Clone, Copy, Debug)]
pub enum CaptureResult {
    Captured(Hotkey),
    Cancelled,
    /// The pressed combo is not usable as a hotkey; capture continues and
    /// the UI shows this hint.
    Rejected(&'static str),
}

/// State shared between the worker thread (hook + tray) and the egui thread.
pub struct Shared {
    pub config: Mutex<Config>,
    /// When true, the hook captures the next key instead of switching layout.
    pub capturing: AtomicBool,
    /// Set by the hook once the user pressed a key while capturing.
    pub capture: Mutex<Option<CaptureResult>>,
    /// Worker thread id, used for PostThreadMessage wakeups.
    pub worker_tid: AtomicU32,
    /// One-off note about the autostart entry (stale path repaired at
    /// startup, or a repair that failed).
    pub autostart_note: Mutex<Option<String>>,
    /// An elevated helper is running: the Autostart window shows a waiting
    /// hint until the helper exits.
    pub autostart_pending: AtomicBool,
}

impl Shared {
    pub fn new(config: Config) -> Self {
        Self {
            config: Mutex::new(config),
            capturing: AtomicBool::new(false),
            capture: Mutex::new(None),
            worker_tid: AtomicU32::new(0),
            autostart_note: Mutex::new(None),
            autostart_pending: AtomicBool::new(false),
        }
    }
}
