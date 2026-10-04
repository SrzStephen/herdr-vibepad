//! Shared low-level hidraw primitives.
//!
//! These helpers unify the near-duplicate `find_device`/`send`/`request`
//! functions that exist in both `agentpad.keys` and `agentpad.led`, plus the
//! directory-scanning shape of `agentpad.daemon`'s `report_node`/`input_nodes`.
//! They know nothing about any particular device's command protocol.

use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::os::unix::io::RawFd;
use std::path::PathBuf;

use nix::poll::{poll, PollFd, PollFlags};
use nix::unistd::{read, write};

/// Find the hidraw device node under `/sys/class/hidraw` whose `device/uevent`
/// contains `vendor_product` (e.g. `"00006D7D:0000DCFC"`) and whose interface
/// path ends with `input_suffix` (e.g. `"/input2\n"`).
///
/// Mirrors `keys.py`/`led.py`'s `find_device()` (interface 2, the vendor
/// config channel) and `daemon.py`'s `report_node()` (interface 1, key
/// reports) — callers pick which interface via `input_suffix`.
pub fn find_hidraw(vendor_product: &str, input_suffix: &str) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir("/sys/class/hidraw")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for d in entries {
        let uevent = match fs::read_to_string(d.join("device/uevent")) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if uevent.contains(vendor_product) && uevent.contains(input_suffix) {
            let name = d.file_name()?.to_str()?;
            return Some(PathBuf::from(format!("/dev/{name}")));
        }
    }
    None
}

/// Find every `/dev/input/eventN` node whose USB vendor/product id (as the
/// lowercase hex strings the kernel exposes under
/// `/sys/class/input/eventN/device/id/{vendor,product}`) matches.
///
/// Mirrors `daemon.py`'s `input_nodes()`.
pub fn find_input_event_nodes(vendor_hex: &str, product_hex: &str) -> Vec<PathBuf> {
    let mut nodes = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/input") {
        for entry in rd.filter_map(|e| e.ok()) {
            let d = entry.path();
            let Some(name) = d.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("event") {
                continue;
            }
            let vendor = fs::read_to_string(d.join("device/id/vendor"));
            let product = fs::read_to_string(d.join("device/id/product"));
            if let (Ok(v), Ok(p)) = (vendor, product) {
                if v.trim() == vendor_hex && p.trim() == product_hex {
                    nodes.push(PathBuf::from(format!("/dev/input/{name}")));
                }
            }
        }
    }
    nodes.sort();
    nodes
}

/// Send one unnumbered 64-byte HID report, `payload` left-padded to 64 bytes
/// with zeros and prefixed with the leading `0x00` report-number byte.
pub fn send(fd: RawFd, payload: &[u8]) -> io::Result<()> {
    let mut frame = [0u8; 65];
    frame[1..1 + payload.len()].copy_from_slice(payload);
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    write(borrowed, &frame).map_err(io::Error::from)?;
    Ok(())
}

/// Send `payload` and wait (up to one second at a time) for a reply that
/// `is_reply` accepts; the device answers one request at a time. `what`
/// names the request for error messages.
pub fn request(
    fd: RawFd,
    payload: &[u8],
    is_reply: impl Fn(&[u8]) -> bool,
    what: &str,
) -> Result<Vec<u8>, String> {
    send(fd, payload).map_err(|e| format!("write failed ({what}): {e}"))?;
    loop {
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let mut fds = [PollFd::new(borrowed, PollFlags::POLLIN)];
        let ready = poll(&mut fds, 1000u16).map_err(|e| format!("poll failed ({what}): {e}"))?;
        if ready == 0 {
            return Err(format!("no reply from device ({what})"));
        }
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let mut buf = [0u8; 64];
        let n = read(borrowed, &mut buf).map_err(|e| format!("read failed ({what}): {e}"))?;
        let r = &buf[..n];
        if is_reply(r) {
            return Ok(r.to_vec());
        }
    }
}
