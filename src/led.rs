//! Control the LEDs on the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc, 16 keys + 3 knobs).
//!
//! Ported from `src/agentpad/led.py`; see that module's docstring for the
//! user-facing protocol description (also reproduced as this crate's
//! `side-keyboard-led` binary's usage text).

use std::io;
use std::os::unix::io::RawFd;
use std::path::PathBuf;

use crate::hid;
use crate::keys::NUM_KEYS;

pub const CUSTOM: u8 = 5;

const VENDOR_PRODUCT: &str = "00006D7D:0000DCFC";
const CONFIG_INPUT_SUFFIX: &str = "/input2\n";

/// Find the pad's vendor config hidraw node (interface 2).
pub fn find_device() -> Result<PathBuf, String> {
    hid::find_hidraw(VENDOR_PRODUCT, CONFIG_INPUT_SUFFIX)
        .ok_or_else(|| "SIDE-KEYBOARD config interface not found".to_string())
}

/// Request the pad's current backlight state and return its 11-byte body
/// (mirrors Python's `r[5:16]`): `[type, 0, mode, brightness, speed,
/// direction, color, 0, h, s, v]`.
pub fn read_state(fd: RawFd) -> Result<[u8; 11], String> {
    let r = hid::request(
        fd,
        &[0x06, 0x0A],
        |r| r.len() >= 16 && r[0] == 0xAA,
        "backlight",
    )
    .map_err(|_| "no backlight reply from device".to_string())?;
    let mut s = [0u8; 11];
    s.copy_from_slice(&r[5..16]);
    Ok(s)
}

/// Write back an 11-byte backlight state (as returned by `read_state`).
pub fn write_state(fd: RawFd, s: &mut [u8; 11]) {
    if s[2] == 0 {
        s[6] = 0;
    }
    let mut payload = vec![0x06, 0x0B, s.len() as u8, 0x00, 0x00];
    payload.extend_from_slice(s);
    hid::send(fd, &payload).expect("hid write failed");
}

/// Set one key's colour (only takes effect in custom mode).
pub fn set_key(fd: RawFd, index: usize, rgb: [u8; 3]) -> io::Result<()> {
    let off = 3 * index;
    let payload = [
        0x06,
        0x14,
        3,
        (off & 0xFF) as u8,
        (off >> 8) as u8,
        0,
        0,
        0,
        rgb[0],
        rgb[1],
        rgb[2],
    ];
    hid::send(fd, &payload)
}

/// Set every key to the same colour, in 56-byte chunks.
pub fn set_all_keys(fd: RawFd, rgb: [u8; 3]) -> io::Result<()> {
    let mut data = Vec::with_capacity(3 * NUM_KEYS);
    for _ in 0..NUM_KEYS {
        data.extend_from_slice(&rgb);
    }
    let mut start = 0usize;
    while start < data.len() {
        let end = (start + 56).min(data.len());
        let chunk = &data[start..end];
        let mut payload = vec![
            0x06,
            0x12,
            (chunk.len() + 3) as u8,
            (start & 0xFF) as u8,
            (start >> 8) as u8,
            0,
            0,
            0,
        ];
        payload.extend_from_slice(chunk);
        hid::send(fd, &payload)?;
        start += 56;
    }
    Ok(())
}

/// Switch the pad into custom (per-key) mode and blank every key, unless
/// it's already there.
pub fn ensure_custom(fd: RawFd, cur: &[u8; 11]) -> io::Result<()> {
    if cur[2] != CUSTOM || cur[3] == 0 {
        let mut new = *cur;
        new[2] = CUSTOM;
        new[3] = 4;
        write_state(fd, &mut new);
        set_all_keys(fd, [0, 0, 0])?;
    }
    Ok(())
}

/// Parse a `#RRGGBB` or `RRGGBB` hex colour into its three bytes.
pub fn parse_rgb(hexstr: &str) -> Result<[u8; 3], String> {
    let v = u32::from_str_radix(hexstr.trim_start_matches('#'), 16)
        .map_err(|_| format!("bad colour {hexstr:?}"))?;
    Ok([
        ((v >> 16) & 0xFF) as u8,
        ((v >> 8) & 0xFF) as u8,
        (v & 0xFF) as u8,
    ])
}
