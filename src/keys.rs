//! Set what each key and knob on the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc) sends.
//!
//! Ported from `src/agentpad/keys.py`; see that module's docstring for the
//! user-facing protocol description (also reproduced as this crate's
//! `side-keyboard-keys` binary's usage text).

use std::env;
use std::ffi::{CStr, CString};
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::PathBuf;

use crate::hid;

pub const NUM_SLOTS: usize = 25; // 16 keys + 3 knobs x 3 actions
pub const NUM_KEYS: usize = 16;
const LAYER: u8 = 0; // the pad reports one layer per profile

pub const TYPE_KEY: u8 = 0x20;
pub const TYPE_CONSUMER: u8 = 0x30;
pub const TYPE_DISABLED: u8 = 0x13;
pub const TYPE_MACRO: u8 = 0x60;
pub const TYPE_FUNCTION: u8 = 0x1F; // actions the pad performs itself

pub const KNOB_PARTS: [&str; 3] = ["press", "right", "left"]; // order of a knob's three slots

const VENDOR_PRODUCT: &str = "00006D7D:0000DCFC";
const CONFIG_INPUT_SUFFIX: &str = "/input2\n";

const MODIFIERS: &[(&str, u8)] = &[
    ("ctrl", 0x01),
    ("shift", 0x02),
    ("alt", 0x04),
    ("super", 0x08),
    ("win", 0x08),
    ("gui", 0x08),
    ("rctrl", 0x10),
    ("rshift", 0x20),
    ("ralt", 0x40),
    ("rsuper", 0x80),
];

const NAMED_USAGES: &[(&str, u8)] = &[
    ("enter", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("minus", 0x2D),
    ("equal", 0x2E),
    ("lbracket", 0x2F),
    ("rbracket", 0x30),
    ("backslash", 0x31),
    ("semicolon", 0x33),
    ("quote", 0x34),
    ("grave", 0x35),
    ("comma", 0x36),
    ("dot", 0x37),
    ("slash", 0x38),
    ("capslock", 0x39),
    ("printscreen", 0x46),
    ("scrolllock", 0x47),
    ("pause", 0x48),
    ("insert", 0x49),
    ("home", 0x4A),
    ("pageup", 0x4B),
    ("delete", 0x4C),
    ("end", 0x4D),
    ("pagedown", 0x4E),
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
    ("menu", 0x65),
];

const CONSUMER: &[(&str, u16)] = &[
    ("volup", 0xE9),
    ("voldown", 0xEA),
    ("mute", 0xE2),
    ("play", 0xCD),
    ("next", 0xB5),
    ("prev", 0xB6),
    ("stop", 0xB7),
];

const FUNCTIONS: &[(&str, u8)] = &[("profileswitch", 0x13)];

/// The HID usage for a single-character key ("a"-"z", "0"-"9") or a function
/// key ("f1"-"f24"), matching `keys.py`'s generated `USAGES` table.
fn generated_usage(key: &str) -> Option<u8> {
    if key.chars().count() == 1 {
        let c = key.chars().next().unwrap();
        if c.is_ascii_lowercase() {
            return Some(0x04 + (c as u8 - b'a'));
        }
        if c == '0' {
            return Some(0x27);
        }
        if c.is_ascii_digit() {
            let n = c.to_digit(10).unwrap() as u8;
            return Some(0x1E + n - 1);
        }
    }
    if let Some(rest) = key.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(0x3A + n - 1);
            }
            if (13..=24).contains(&n) {
                return Some(0x68 + (n - 13));
            }
        }
    }
    None
}

fn usage_for(key: &str) -> Option<u8> {
    generated_usage(key).or_else(|| {
        NAMED_USAGES
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    })
}

fn usage_name(v: u8) -> Option<String> {
    if (0x04..=0x1D).contains(&v) {
        return Some(((b'a' + (v - 0x04)) as char).to_string());
    }
    if (0x1E..=0x26).contains(&v) {
        return Some((1 + (v - 0x1E)).to_string());
    }
    if v == 0x27 {
        return Some("0".to_string());
    }
    if (0x3A..=0x45).contains(&v) {
        return Some(format!("f{}", 1 + (v - 0x3A)));
    }
    if (0x68..=0x73).contains(&v) {
        return Some(format!("f{}", 13 + (v - 0x68)));
    }
    NAMED_USAGES
        .iter()
        .find(|(_, val)| *val == v)
        .map(|(k, _)| k.to_string())
}

/// Parse a key name with optional `+`-joined modifiers (e.g. `"ctrl+shift+t"`)
/// into a 4-byte slot entry `[type, byte1, byte2, byte3]`.
pub fn parse_key(text: &str) -> Result<[u8; 4], String> {
    let lower = text.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    let mut mods: u8 = 0;
    for m in &parts[..parts.len() - 1] {
        match MODIFIERS.iter().find(|(k, _)| k == m) {
            Some((_, v)) => mods |= v,
            None => return Err(format!("unknown modifier {m:?}")),
        }
    }
    let key = parts[parts.len() - 1];
    if mods == 0 {
        if let Some((_, v)) = FUNCTIONS.iter().find(|(k, _)| *k == key) {
            return Ok([TYPE_FUNCTION, *v, 0, 0]);
        }
        if let Some((_, code)) = CONSUMER.iter().find(|(k, _)| *k == key) {
            return Ok([TYPE_CONSUMER, (*code & 0xFF) as u8, (*code >> 8) as u8, 0]);
        }
    }
    if let Some(v) = usage_for(key) {
        return Ok([TYPE_KEY, mods, v, 0]);
    }
    if let Some(hex) = key.strip_prefix("0x") {
        if let Ok(v) = u8::from_str_radix(hex, 16) {
            return Ok([TYPE_KEY, mods, v, 0]);
        }
    }
    Err(format!("unknown key {key:?}"))
}

/// Render a 4-byte slot entry back into the key name syntax `parse_key` accepts.
pub fn describe(entry: [u8; 4]) -> String {
    let [t, c1, c2, c3] = entry;
    if t == TYPE_KEY {
        let name = usage_name(c2).unwrap_or_else(|| format!("0x{c2:02x}"));
        let mut parts: Vec<String> = MODIFIERS
            .iter()
            .filter(|(k, v)| c1 & v != 0 && *k != "win" && *k != "gui")
            .map(|(k, _)| k.to_string())
            .collect();
        parts.push(name);
        return parts.join("+");
    }
    if t == TYPE_CONSUMER {
        let code = (c1 as u16) | ((c2 as u16) << 8);
        return CONSUMER
            .iter()
            .find(|(_, v)| *v == code)
            .map(|(k, _)| k.to_string())
            .unwrap_or_else(|| format!("media 0x{code:04x}"));
    }
    if t == TYPE_FUNCTION {
        return FUNCTIONS
            .iter()
            .find(|(_, v)| *v == c1)
            .map(|(k, _)| k.to_string())
            .unwrap_or_else(|| format!("function 0x{c1:02x}"));
    }
    if t == TYPE_MACRO {
        return format!("macro M{c1}");
    }
    if t == TYPE_DISABLED {
        return "(disabled)".to_string();
    }
    format!("type 0x{t:02x}: {c1:02x} {c2:02x} {c3:02x}")
}

/// Parse a slot name: `"0"`-`"15"` for keys, or `"knob1"`-`"knob3"` with
/// `.press`/`.left`/`.right`.
pub fn parse_slot(text: &str) -> Result<usize, String> {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = text.parse::<usize>() {
            if n < NUM_KEYS {
                return Ok(n);
            }
        }
    }
    if let Some(rest) = text.strip_prefix("knob") {
        if let Some(dot) = rest.find('.') {
            let knob = &rest[..dot];
            let part = &rest[dot + 1..];
            if ["1", "2", "3"].contains(&knob) {
                if let Some(idx) = KNOB_PARTS.iter().position(|p| *p == part) {
                    let k: usize = knob.parse().unwrap();
                    return Ok(NUM_KEYS + 3 * (k - 1) + idx);
                }
            }
        }
    }
    Err(format!(
        "bad slot {text:?}: use 0-15 or knob1-3.press/left/right"
    ))
}

/// The human-readable name of a slot, e.g. `"key 3"` or `"knob1.press"`.
pub fn slot_name(slot: usize) -> String {
    if slot < NUM_KEYS {
        return format!("key {slot}");
    }
    let k = (slot - NUM_KEYS) / 3;
    let p = (slot - NUM_KEYS) % 3;
    format!("knob{}.{}", k + 1, KNOB_PARTS[p])
}

/// Find the pad's vendor config hidraw node (interface 2).
pub fn find_device() -> Result<PathBuf, String> {
    hid::find_hidraw(VENDOR_PRODUCT, CONFIG_INPUT_SUFFIX)
        .ok_or_else(|| "SIDE-KEYBOARD config interface not found".to_string())
}

/// `(active profile, profile count)`.
pub fn get_profile(fd: RawFd) -> Result<(u8, u8), String> {
    let r = hid::request(
        fd,
        &[0x06, 0x05],
        |r| r.len() >= 17 && r[0] == 0xAA && r[1] == 0x05,
        "device info",
    )?;
    Ok((r[16], r[15]))
}

pub fn select_profile(fd: RawFd, n: u8) -> Result<(), String> {
    hid::request(
        fd,
        &[0x06, 0xFB, n],
        |r| r.len() >= 8 && r[0] == 0xAA && r[1] == 0xFB,
        &format!("select profile {n}"),
    )?;
    Ok(())
}

/// Read the active layer's full slot table: `NUM_SLOTS` 4-byte entries.
pub fn read_table(fd: RawFd) -> Result<Vec<[u8; 4]>, String> {
    let mut raw: Vec<u8> = Vec::new();
    let mut off: usize = 0;
    while off < 4 * NUM_SLOTS {
        loop {
            let payload = [
                0x06,
                0x08,
                0x3A,
                (off & 0xFF) as u8,
                (off >> 8) as u8,
                0,
                LAYER,
            ];
            // The pad answers a 0x08 read with an 0x07 header.
            let r = hid::request(
                fd,
                &payload,
                |r| r.len() >= 8 && r[0] == 0xAA && (r[1] == 0x07 || r[1] == 0x08),
                "read keys",
            )?;
            if r.len() >= 5 && r[3] == (off & 0xFF) as u8 && r[4] == (off >> 8) as u8 {
                let end = r.len().min(64);
                raw.extend_from_slice(&r[8..end]);
                break;
            }
        }
        off += 56;
    }
    raw.truncate(4 * NUM_SLOTS);
    Ok(raw.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect())
}

pub fn write_slot(fd: RawFd, slot: usize, entry: [u8; 4]) -> Result<(), String> {
    let off = 4 * slot;
    // Back-to-back writes without waiting for the ack corrupt the table.
    let mut payload = vec![
        0x06,
        0x10,
        0x07,
        (off & 0xFF) as u8,
        (off >> 8) as u8,
        0,
        LAYER,
        0,
    ];
    payload.extend_from_slice(&entry);
    hid::request(
        fd,
        &payload,
        |r| r.len() >= 8 && r[0] == 0xAA && r[1] == 0x10,
        &format!("write {}", slot_name(slot)),
    )?;
    Ok(())
}

fn home_dir() -> String {
    let sudo_user = env::var("SUDO_USER").unwrap_or_default();
    if !sudo_user.is_empty() {
        if let Ok(cname) = CString::new(sudo_user) {
            unsafe {
                let pw = libc::getpwnam(cname.as_ptr());
                if !pw.is_null() {
                    return CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
                }
            }
        }
    }
    env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

pub fn backup_path(profile: u8) -> PathBuf {
    PathBuf::from(format!(
        "{}/.side-keyboard-keys-backup-p{profile}.json",
        home_dir()
    ))
}

/// Save `table` (the profile's current mapping) to its backup file, unless
/// one already exists.
pub fn save_backup(table: &[[u8; 4]], profile: u8) -> io::Result<()> {
    let path = backup_path(profile);
    if path.exists() {
        return Ok(());
    }
    let table_json: Vec<Vec<u8>> = table.iter().map(|e| e.to_vec()).collect();
    let data = serde_json::json!({"profile": profile, "layer": LAYER, "table": table_json});
    fs::write(&path, serde_json::to_vec(&data).map_err(io::Error::other)?)?;
    if let (Ok(uid), Ok(gid)) = (env::var("SUDO_UID"), env::var("SUDO_GID")) {
        if let (Ok(uid), Ok(gid)) = (uid.parse::<u32>(), gid.parse::<u32>()) {
            if let Ok(cpath) = CString::new(path.to_string_lossy().as_bytes()) {
                unsafe {
                    libc::chown(cpath.as_ptr(), uid, gid);
                }
            }
        }
    }
    println!(
        "saved profile {profile}'s original mapping to {}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_letter() {
        assert_eq!(parse_key("a").unwrap(), [TYPE_KEY, 0, 0x04, 0]);
    }

    #[test]
    fn parses_modifiers() {
        assert_eq!(
            parse_key("ctrl+shift+t").unwrap(),
            [TYPE_KEY, 0x03, 0x17, 0]
        );
    }

    #[test]
    fn parses_f13_and_f24() {
        assert_eq!(parse_key("f13").unwrap(), [TYPE_KEY, 0, 0x68, 0]);
        assert_eq!(parse_key("f24").unwrap(), [TYPE_KEY, 0, 0x73, 0]);
    }

    #[test]
    fn parses_consumer_and_function_keys() {
        assert_eq!(parse_key("volup").unwrap(), [TYPE_CONSUMER, 0xE9, 0, 0]);
        assert_eq!(
            parse_key("profileswitch").unwrap(),
            [TYPE_FUNCTION, 0x13, 0, 0]
        );
    }

    #[test]
    fn describe_round_trips() {
        for text in [
            "a",
            "ctrl+shift+t",
            "shift+f18",
            "esc",
            "mute",
            "profileswitch",
        ] {
            assert_eq!(describe(parse_key(text).unwrap()), text);
        }
    }

    #[test]
    fn parses_and_names_slots() {
        for (text, slot) in [
            ("0", 0),
            ("15", 15),
            ("knob1.press", 16),
            ("knob3.left", 24),
        ] {
            assert_eq!(parse_slot(text).unwrap(), slot);
            assert_eq!(
                slot_name(slot),
                if text.chars().all(|c| c.is_ascii_digit()) {
                    format!("key {text}")
                } else {
                    text.to_string()
                }
            );
        }
    }

    #[test]
    fn bad_key_is_an_error() {
        assert!(parse_key("hyper+q").is_err());
    }
}
