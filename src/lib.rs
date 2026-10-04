//! agentpad: HID protocol, herdr client and daemon logic for the SDINNOVATION SIDE-KEYBOARD.

use std::sync::Mutex;

pub mod daemon;
pub mod herdr;
pub mod hid;
pub mod keys;
pub mod led;

static LAST_LOG: Mutex<Option<String>> = Mutex::new(None);

/// Print a message to stdout (the systemd service's log capture depends on
/// stdout, like Python's `print(..., flush=True)`); with `once`, skip it if
/// it repeats the immediately previous message.
///
/// Ports `daemon.py`'s `log()`.
pub fn log(msg: &str, once: bool) {
    let mut last = LAST_LOG.lock().expect("LAST_LOG mutex poisoned");
    if once && last.as_deref() == Some(msg) {
        return;
    }
    *last = Some(msg.to_string());
    println!("{msg}");
}
