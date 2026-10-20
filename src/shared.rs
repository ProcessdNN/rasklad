use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Mutex;

use crate::config::Config;
use crate::hotkey::Hotkey;

/// Result of a hotkey-capture session in the keyboard hook.
#[derive(Clone, Copy, Debug)]
pub enum CaptureResult {
    Captured(Hotkey),
    Cancelled,
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
}

impl Shared {
    pub fn new(config: Config) -> Self {
        Self {
            config: Mutex::new(config),
            capturing: AtomicBool::new(false),
            capture: Mutex::new(None),
            worker_tid: AtomicU32::new(0),
        }
    }
}
