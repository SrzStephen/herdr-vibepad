//! Drive herdr workspaces and agents from the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc).
//!
//! Ported from `src/agentpad/daemon.py`; see that module's docstring for the
//! user-facing behaviour description. This file currently carries the
//! daemon's core plumbing: key/LED position mapping, the pad's hidraw slot
//! protocol table, the `Pad` hardware handle and its event loop, and `State`
//! (herdr's workspace/agent snapshot). `AgentPad` (the actual key-handling
//! behaviour) is built on top of this in a later module.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{open, OFlag};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::stat::Mode;

use crate::{herdr, hid, keys, led};

pub const PAD_PROFILE: u8 = 5;
pub const LED_HW_BRIGHTNESS: u8 = 4; // the pad's own brightness (0-4); colours are scaled in software

const VENDOR_PRODUCT: &str = "00006D7D:0000DCFC";
const REPORT_INPUT_SUFFIX: &str = "/input1\n";
const EVIOCGRAB: libc::c_ulong = 0x40044590;

/// Physical position (left to right, top to bottom) of pad key/LED index `i`.
///
/// The pad numbers keys and LEDs bottom to top, left column first.
pub fn position(i: usize) -> usize {
    (3 - i % 4) * 4 + i / 4
}

/// What each pad slot sends: keys 0-15, then knob1-3 x press/right/left.
fn slot_keys() -> Vec<String> {
    let mut v: Vec<String> = (13..=24).map(|n| format!("f{n}")).collect();
    v.extend((13..=24).map(|n| format!("shift+f{n}")));
    v.push("ctrl+f13".to_string());
    v
}

/// The 4-byte slot entry each pad slot is programmed to send, in slot order.
fn slot_entries() -> &'static Vec<[u8; 4]> {
    static ENTRIES: OnceLock<Vec<[u8; 4]>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        slot_keys()
            .iter()
            .map(|k| keys::parse_key(k).expect("slot_keys() entries are always valid key names"))
            .collect()
    })
}

/// The kernel drops F13-F24 from this pad's keyboard interface, so key reports
/// are read raw from hidraw: `[modifiers, 0, usage x 6]`. Maps
/// `(modifiers, usage) -> slot`.
fn code_to_slot() -> &'static HashMap<(u8, u8), usize> {
    static TABLE: OnceLock<HashMap<(u8, u8), usize>> = OnceLock::new();
    TABLE.get_or_init(|| {
        slot_entries()
            .iter()
            .enumerate()
            .map(|(slot, e)| ((e[1], e[2]), slot))
            .collect()
    })
}

// ---------------------------------------------------------------- herdr state

pub struct Workspace {
    pub workspace_id: String,
    pub number: u32,
}

/// What the pad needs to know about herdr right now.
pub struct State {
    pub workspaces: Vec<Workspace>,
    pub workspace: Option<String>,
    pub all_agents: Vec<String>, // every workspace, in workspace order
    pub agents: Vec<String>,     // just the focused workspace's agents
    pub status: HashMap<String, String>,
    pub active: Option<String>,
}

/// "w1:t2" -> 2, "w1:p3" -> 3
fn id_number(ident: &str) -> u32 {
    ident
        .rsplit(':')
        .next()
        .and_then(|tail| tail.get(1..))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

impl State {
    pub fn fetch(sock_path: &Path) -> State {
        let ws = herdr::call(sock_path, "workspace.list", serde_json::json!({}));
        let ag = herdr::call(sock_path, "agent.list", serde_json::json!({}));

        let raw_workspaces: Vec<serde_json::Value> = ws
            .as_ref()
            .and_then(|v| v.get("workspaces"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut workspaces: Vec<Workspace> = raw_workspaces
            .iter()
            .filter_map(|w| {
                Some(Workspace {
                    workspace_id: w.get("workspace_id")?.as_str()?.to_string(),
                    number: w.get("number")?.as_u64()? as u32,
                })
            })
            .collect();
        workspaces.sort_by_key(|w| w.number);

        let workspace = raw_workspaces
            .iter()
            .find(|w| w.get("focused").and_then(|f| f.as_bool()) == Some(true))
            .and_then(|w| w.get("workspace_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let number: HashMap<&str, u32> = raw_workspaces
            .iter()
            .filter_map(|w| {
                Some((
                    w.get("workspace_id")?.as_str()?,
                    w.get("number")?.as_u64()? as u32,
                ))
            })
            .collect();

        let mut agents: Vec<serde_json::Value> = ag
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        agents.sort_by_key(|a| {
            let ws_id = a.get("workspace_id").and_then(|v| v.as_str()).unwrap_or("");
            let n = number.get(ws_id).copied().unwrap_or(0);
            let tab = a
                .get("tab_id")
                .and_then(|v| v.as_str())
                .map(id_number)
                .unwrap_or(0);
            let pane = a
                .get("pane_id")
                .and_then(|v| v.as_str())
                .map(id_number)
                .unwrap_or(0);
            (n, tab, pane)
        });

        let all_agents: Vec<String> = agents
            .iter()
            .filter_map(|a| {
                a.get("pane_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();

        let agents_here: Vec<String> = agents
            .iter()
            .filter(|a| a.get("workspace_id").and_then(|v| v.as_str()) == workspace.as_deref())
            .filter_map(|a| {
                a.get("pane_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();

        let status: HashMap<String, String> = agents
            .iter()
            .filter_map(|a| {
                let pane = a.get("pane_id")?.as_str()?.to_string();
                let st = a.get("agent_status")?.as_str()?.to_string();
                Some((pane, st))
            })
            .collect();

        let active = agents
            .iter()
            .find(|a| a.get("focused").and_then(|f| f.as_bool()) == Some(true))
            .and_then(|a| a.get("pane_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        State {
            workspaces,
            workspace,
            all_agents,
            agents: agents_here,
            status,
            active,
        }
    }
}

// ---------------------------------------------------------------- pad

#[derive(Debug)]
pub struct PadGone(pub String);

impl From<String> for PadGone {
    fn from(s: String) -> Self {
        PadGone(s)
    }
}

impl std::fmt::Display for PadGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PadGone {}

pub struct Pad {
    hid: OwnedFd,         // the vendor config interface: commands and their replies
    reports: OwnedFd,     // the raw key-report interface (F13-F24 codes)
    inputs: Vec<OwnedFd>, // grabbed only so the pad's codes don't reach the desktop
    down: Vec<u8>,        // usages held in the last key report
    frame: Option<[[u8; 3]; 16]>,
}

fn open_rdwr(path: &Path) -> Result<OwnedFd, PadGone> {
    open(path, OFlag::O_RDWR, Mode::empty())
        .map_err(|e| PadGone(format!("open {}: {e}", path.display())))
}

fn open_rdonly_nonblock(path: &Path) -> Result<OwnedFd, PadGone> {
    open(path, OFlag::O_RDONLY | OFlag::O_NONBLOCK, Mode::empty())
        .map_err(|e| PadGone(format!("open {}: {e}", path.display())))
}

/// Write an LED state, converting a panic from `led::write_state`'s internal
/// `.expect()` (see that function's docs: it panics rather than propagating an
/// I/O error) into a retryable `PadGone` instead of crashing the daemon.
fn write_led_state(fd: RawFd, state: &mut [u8; 11]) -> Result<(), PadGone> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| led::write_state(fd, state)))
        .map_err(|_| PadGone("LED write failed (device I/O error)".to_string()))
}

impl Pad {
    pub fn open() -> Result<Pad, PadGone> {
        // The vendor config interface: used for every command (profile
        // select, slot table read/write, LED state).
        let hid_path = keys::find_device()?;
        let hid = open_rdwr(&hid_path)?;

        // The raw key-report interface.
        let report_path = hid::find_hidraw(VENDOR_PRODUCT, REPORT_INPUT_SUFFIX)
            .ok_or_else(|| PadGone("pad key interface not found".to_string()))?;
        let reports = open_rdonly_nonblock(&report_path)?;

        let mut inputs = Vec::new();
        for path in hid::find_input_event_nodes("6d7d", "dcfc") {
            let fd = open_rdonly_nonblock(&path)?;
            // A failed grab is non-fatal, matching Python, which doesn't check it either.
            unsafe {
                libc::ioctl(fd.as_raw_fd(), EVIOCGRAB, 1);
            }
            inputs.push(fd);
        }
        if inputs.is_empty() {
            // e.g. WSL without evdev; there's no desktop to shield.
            crate::log(
                "pad input devices not found; its keys are not grabbed",
                true,
            );
        }

        let mut pad = Pad {
            hid,
            reports,
            inputs,
            down: Vec::new(),
            frame: None,
        };
        pad.prepare()?;
        Ok(pad)
    }

    /// Discard any bytes currently waiting to be read from `hid` (command
    /// replies we're no longer interested in).
    fn drain(&self) {
        loop {
            let raw = self.hid.as_raw_fd();
            let borrowed = unsafe { BorrowedFd::borrow_raw(raw) };
            let mut fds = [PollFd::new(borrowed, PollFlags::POLLIN)];
            match poll(&mut fds, PollTimeout::ZERO) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let borrowed = unsafe { BorrowedFd::borrow_raw(raw) };
            let mut buf = [0u8; 64];
            if nix::unistd::read(borrowed, &mut buf).is_err() {
                break;
            }
        }
    }

    /// Select our profile, make its slots send our codes, set LEDs to per-key
    /// custom mode at [`LED_HW_BRIGHTNESS`].
    fn prepare(&mut self) -> Result<(), PadGone> {
        self.drain();
        let (active, _) = keys::get_profile(self.hid.as_raw_fd())?;
        if active != PAD_PROFILE {
            // The pad sometimes switches without replying; ignore a failed
            // request here and verify the switch actually happened below.
            let _ = keys::select_profile(self.hid.as_raw_fd(), PAD_PROFILE);
            self.drain();
            if keys::get_profile(self.hid.as_raw_fd())?.0 != PAD_PROFILE {
                return Err(PadGone(format!(
                    "pad did not switch to profile {PAD_PROFILE}"
                )));
            }
            crate::log(
                &format!("pad switched from profile {active} to {PAD_PROFILE}"),
                false,
            );
        }

        let table = keys::read_table(self.hid.as_raw_fd())?;
        let wrong: Vec<usize> = slot_entries()
            .iter()
            .enumerate()
            .filter_map(|(slot, want)| (table[slot] != *want).then_some(slot))
            .collect();
        if !wrong.is_empty() {
            keys::save_backup(&table, PAD_PROFILE).map_err(|e| PadGone(e.to_string()))?;
            for &slot in &wrong {
                keys::write_slot(self.hid.as_raw_fd(), slot, slot_entries()[slot])?;
            }
            crate::log(
                &format!(
                    "programmed {} slot(s) of profile {PAD_PROFILE}",
                    wrong.len()
                ),
                false,
            );
        }

        self.drain();
        let mut state = led::read_state(self.hid.as_raw_fd())?;
        if state[2] != led::CUSTOM || state[3] != LED_HW_BRIGHTNESS {
            state[2] = led::CUSTOM;
            state[3] = LED_HW_BRIGHTNESS;
            write_led_state(self.hid.as_raw_fd(), &mut state)?;
        }
        Ok(())
    }

    /// Wait up to `timeout` and return the pad slots pressed meanwhile.
    pub fn events(&mut self, timeout: Duration) -> Result<Vec<usize>, PadGone> {
        let fds_raw: Vec<RawFd> = std::iter::once(self.hid.as_raw_fd())
            .chain(std::iter::once(self.reports.as_raw_fd()))
            .chain(self.inputs.iter().map(|f| f.as_raw_fd()))
            .collect();

        let poll_timeout = PollTimeout::try_from(timeout).unwrap_or(PollTimeout::MAX);
        let mut pollfds: Vec<PollFd> = fds_raw
            .iter()
            .map(|&fd| {
                let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
                PollFd::new(borrowed, PollFlags::POLLIN)
            })
            .collect();

        loop {
            match poll(&mut pollfds, poll_timeout) {
                Ok(_) => break,
                Err(Errno::EINTR) => continue,
                Err(e) => return Err(PadGone(format!("poll failed: {e}"))),
            }
        }

        let mut slots = Vec::new();
        let mut buf = [0u8; 4096];
        for (i, pfd) in pollfds.iter().enumerate() {
            if pfd.any() != Some(true) {
                continue;
            }
            let borrowed = unsafe { BorrowedFd::borrow_raw(fds_raw[i]) };
            // Command replies and grabbed events are discarded (only the
            // reports fd, index 1, carries key data).
            let n = match nix::unistd::read(borrowed, &mut buf) {
                Ok(n) => n,
                // A spurious poll wakeup with nothing to read: not an error.
                // EWOULDBLOCK == EAGAIN on Linux.
                Err(Errno::EAGAIN) => continue,
                Err(Errno::ENODEV | Errno::EIO | Errno::EBADF) => {
                    return Err(PadGone("pad disconnected".to_string()))
                }
                Err(e) => return Err(PadGone(format!("pad read failed: {e}"))),
            };
            if i != 1 || n < 3 {
                continue;
            }
            let data = &buf[..n];
            let usages: Vec<u8> = data[2..n.min(8)]
                .iter()
                .copied()
                .filter(|&u| u != 0)
                .collect();
            for &u in &usages {
                if !self.down.contains(&u) {
                    if let Some(&slot) = code_to_slot().get(&(data[0], u)) {
                        slots.push(slot);
                    }
                }
            }
            self.down = usages;
        }
        Ok(slots)
    }

    /// Send `colors` (indexed by physical position) to the pad's per-key LEDs,
    /// unless it's the frame already showing.
    pub fn show(&mut self, colors: &[[u8; 3]; 16]) {
        if self.frame.as_ref() == Some(colors) {
            return;
        }
        let mut data = Vec::with_capacity(3 * keys::NUM_KEYS);
        for i in 0..keys::NUM_KEYS {
            data.extend_from_slice(&colors[position(i)]);
        }
        // Same bulk per-key write as led::set_all_keys; 48 bytes fit one frame.
        let mut payload = vec![0x06u8, 0x12, (data.len() + 3) as u8, 0, 0, 0, 0, 0];
        payload.extend_from_slice(&data);
        hid::send(self.hid.as_raw_fd(), &payload).expect("hid write failed");
        self.frame = Some(*colors);
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // `hid`, `reports`, and every fd in `inputs` are `OwnedFd`; Rust closes
        // each one automatically, exactly once, here on every exit path
        // (normal drop, or unwinding through a `?` in `Pad::open`).
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_maps_to_a_distinct_position() {
        let mut positions: Vec<usize> = (0..keys::NUM_KEYS).map(position).collect();
        positions.sort();
        assert_eq!(positions, (0..keys::NUM_KEYS).collect::<Vec<_>>());
    }

    #[test]
    fn position_matches_the_pad_wiring() {
        for (index, expected) in [(3, 0), (12, 15), (0, 12), (15, 3)] {
            assert_eq!(position(index), expected);
        }
    }

    #[test]
    fn every_pad_slot_has_a_distinct_code() {
        assert_eq!(code_to_slot().len(), keys::NUM_SLOTS);
    }

    fn feed(reports: &[&str]) -> Vec<usize> {
        let (hid_r, _hid_w) = nix::unistd::pipe().unwrap();
        let (rep_r, rep_w) = nix::unistd::pipe().unwrap();
        // set rep_r non-blocking, like the real hidraw node (os.set_blocking(rep_r, False) in Python)
        nix::fcntl::fcntl(
            &rep_r,
            nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
        )
        .unwrap();
        let mut pad = Pad {
            hid: hid_r,
            reports: rep_r,
            inputs: vec![],
            down: vec![],
            frame: None,
        };
        let mut slots = vec![];
        for report in reports {
            let bytes: Vec<u8> = report
                .split_whitespace()
                .flat_map(|pair| {
                    (0..pair.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&pair[i..i + 2], 16).unwrap())
                })
                .collect();
            nix::unistd::write(&rep_w, &bytes).unwrap();
            slots.extend(pad.events(std::time::Duration::ZERO).unwrap());
        }
        slots
    }

    #[test]
    fn decodes_reports_captured_from_the_pad() {
        let reports = [
            "0200 6c00 0000 0000",
            "0000 0000 0000 0000",
            "0200 6f00 0000 0000",
            "0200 6f72 0000 0000",
            "0000 6f72 0000 0000",
            "0000 0000 0000 0000",
            "0200 6d00 0000 0000",
            "0000 0000 0000 0000",
            "0100 6800 0000 0000",
            "0000 0000 0000 0000",
            "0000 6b00 0000 0000",
            "0000 0000 0000 0000",
            "0200 6800 0000 0000",
        ];
        assert_eq!(feed(&reports), vec![16, 19, 22, 17, 24, 3, 12]);
    }

    #[test]
    fn a_held_key_counts_once() {
        assert_eq!(
            feed(&[
                "0000 6800 0000 0000",
                "0000 6800 0000 0000",
                "0000 6800 0000 0000"
            ]),
            vec![0]
        );
    }

    #[test]
    fn a_short_report_is_ignored_not_a_panic() {
        assert_eq!(feed(&["0000", "0000 6800 0000 0000"]), vec![0]);
    }
}
