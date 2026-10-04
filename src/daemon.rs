//! Drive herdr workspaces and agents from the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc).
//!
//! Ported from `src/agentpad/daemon.py`. Holds key/LED position mapping, the
//! `Pad` hardware handle, `State` (herdr's snapshot) and `VibePad`, the
//! key-handling behaviour and event loop.

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
            // A failed grab (EBUSY: another instance holds it) is fatal: this is
            // the implicit single-instance guard.
            if unsafe { libc::ioctl(fd.as_raw_fd(), EVIOCGRAB, 1) } < 0 {
                return Err(PadGone(format!(
                    "grab {}: {}",
                    path.display(),
                    Errno::last()
                )));
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
    pub fn show(&mut self, colors: &[[u8; 3]; 16]) -> Result<(), PadGone> {
        if self.frame.as_ref() == Some(colors) {
            return Ok(());
        }
        let mut data = Vec::with_capacity(3 * keys::NUM_KEYS);
        for i in 0..keys::NUM_KEYS {
            data.extend_from_slice(&colors[position(i)]);
        }
        // Same bulk per-key write as led::set_all_keys; 48 bytes fit one frame.
        let mut payload = vec![0x06u8, 0x12, (data.len() + 3) as u8, 0, 0, 0, 0, 0];
        payload.extend_from_slice(&data);
        hid::send(self.hid.as_raw_fd(), &payload).map_err(|e| match e.raw_os_error() {
            Some(code) if code == libc::ENODEV || code == libc::EIO => {
                PadGone("pad disconnected".to_string())
            }
            _ => PadGone(format!("hid write failed: {e}")),
        })?;
        self.frame = Some(*colors);
        Ok(())
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // `hid`, `reports`, and every fd in `inputs` are `OwnedFd`; Rust closes
        // each one automatically, exactly once, here on every exit path
        // (normal drop, or unwinding through a `?` in `Pad::open`).
    }
}

// ---------------------------------------------------------------- behaviour

use std::path::PathBuf;
use std::time::Instant;

pub const AGENT_KEYS: usize = 12;
/// layer -> herdr key names for keys 12-15, `None` = unmapped.
pub const BOTTOM_KEYS: [[Option<&str>; 4]; 3] = [
    [Some("1"), Some("2"), Some("3"), Some("esc")],
    [Some("1"), Some("2"), Some("3"), Some("esc")],
    [Some("y"), Some("n"), Some("t"), Some("esc")],
];
pub const LAYER_COLORS: [(u8, u8, u8); 3] = [(255, 50, 0), (0, 60, 255), (150, 0, 255)];
/// The agent each layer's keys suit.
pub const LAYER_NAMES: [&str; 3] = ["Claude", "Codex", "Kiro"];
/// Bottom row, as a fraction of the layer colour.
pub const BOTTOM_BRIGHTNESS: f64 = 0.2;
pub const OFF: (u8, u8, u8) = (0, 0, 0);
/// Agents other than the active one are this many times dimmer.
pub const INACTIVE_DIM: u32 = 5;
/// Seconds per on/off half of the blocked flash.
pub const FLASH: f64 = 0.5;
/// Knob 1 presses that toggle all-workspaces mode...
pub const TRIPLE_PRESS: usize = 3;
/// ...within this many seconds.
pub const TRIPLE_PRESS_WINDOW: f64 = 1.0;
/// Knob 3: percentage points per click.
pub const BRIGHTNESS_STEP: i32 = 5;
/// So the pad never looks switched off.
pub const BRIGHTNESS_MIN: i32 = 5;
/// Seconds between herdr state polls in [`VibePad::run`].
const POLL: f64 = 0.25;

/// herdr `agent_status` -> colour; anything else (including no status at
/// all) shows as idle.
pub fn status_color(status: &str) -> (u8, u8, u8) {
    match status {
        "working" => (255, 160, 0),
        "blocked" => (255, 0, 0), // waiting on an approval or question; flashes
        "done" => (0, 255, 0),    // finished and not yet looked at
        _ => (255, 255, 255),     // idle, and anything unrecognized
    }
}

/// Python's `round()`: round-half-to-even. `f64::round()` rounds half away
/// from zero instead, which only differs from Python at an exact `.5` tie
/// (and even then only when the integer part below the tie is odd), so only
/// that case needs special handling.
pub fn round_half_even(x: f64) -> i64 {
    let floor = x.floor();
    if x - floor == 0.5 {
        let floor_i = floor as i64;
        if floor_i % 2 == 0 {
            floor_i
        } else {
            floor_i + 1
        }
    } else {
        x.round() as i64
    }
}

/// `items[index_of(current) + delta]`, wrapping; `items[0]`/`items[-1]` if
/// `current` isn't in `items` (stepping right/left respectively); `None` if
/// `items` is empty.
fn step<T: PartialEq + Clone>(items: &[T], current: Option<&T>, delta: i64) -> Option<T> {
    if items.is_empty() {
        return None;
    }
    match current.and_then(|c| items.iter().position(|i| i == c)) {
        None => Some(if delta > 0 {
            items[0].clone()
        } else {
            items[items.len() - 1].clone()
        }),
        Some(idx) => {
            let len = items.len() as i64;
            let new_idx = (idx as i64 + delta).rem_euclid(len) as usize;
            Some(items[new_idx].clone())
        }
    }
}

fn load_brightness(path: &Path) -> i32 {
    match std::fs::read_to_string(path) {
        Ok(s) => match s.trim().parse::<i32>() {
            Ok(n) => n.clamp(BRIGHTNESS_MIN, 100),
            Err(_) => 100,
        },
        Err(_) => 100,
    }
}

fn save_brightness(path: &Path, pct: i32) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(path, format!("{pct}\n")) {
        crate::log(&format!("can't save brightness: {e}"), false);
    }
}

/// The pad's key-handling behaviour: layer, brightness, all-workspaces mode,
/// and translating pad slots/herdr state into herdr calls and LED colours.
pub struct VibePad {
    pub layer: u8,
    pub brightness: i32,
    /// Agent keys cover every workspace, not just the focused one.
    pub all_workspaces: bool,
    pub knob1_presses: Vec<f64>,
    pad: Option<Pad>,
    sock_path: PathBuf,
    brightness_file: PathBuf,
}

impl VibePad {
    pub fn new(pad: Option<Pad>, sock_path: PathBuf, brightness_file: PathBuf) -> VibePad {
        let brightness = load_brightness(&brightness_file);
        VibePad {
            layer: 1,
            brightness,
            all_workspaces: false,
            knob1_presses: Vec::new(),
            pad,
            sock_path,
            brightness_file,
        }
    }

    /// The agents the top three rows stand for, in key order.
    fn keyed_agents<'a>(&self, st: &'a State) -> &'a [String] {
        if self.all_workspaces {
            &st.all_agents
        } else {
            &st.agents
        }
    }

    pub fn press(&mut self, slot: usize, st: &State, now: f64) {
        if slot < keys::NUM_KEYS {
            let pos = position(slot);
            crate::log(
                &format!("key row {} column {}", pos / 4 + 1, pos % 4 + 1),
                false,
            );
            self.key(pos, st);
        } else {
            let rel = slot - keys::NUM_KEYS;
            let (knob, part) = (rel / 3, rel % 3);
            self.knob((knob + 1) as u8, keys::KNOB_PARTS[part], st, now);
        }
    }

    /// `pos`: physical key position, left to right, top to bottom.
    fn key(&mut self, pos: usize, st: &State) {
        if pos < AGENT_KEYS {
            let agents = self.keyed_agents(st);
            if let Some(agent) = agents.get(pos) {
                // Also focuses its workspace.
                herdr::call(
                    &self.sock_path,
                    "agent.focus",
                    serde_json::json!({"target": agent}),
                );
            }
        } else if let Some(key) = BOTTOM_KEYS[(self.layer - 1) as usize][pos - AGENT_KEYS] {
            if let Some(active) = &st.active {
                // agent.send_keys only takes named agents; the pane works for any.
                herdr::call(
                    &self.sock_path,
                    "pane.send_keys",
                    serde_json::json!({"pane_id": active, "keys": [key]}),
                );
            }
        }
    }

    fn knob(&mut self, n: u8, action: &str, st: &State, now: f64) {
        if action == "press" {
            self.layer = n;
            crate::log(
                &format!(
                    "layer {} ({} mode)",
                    self.layer,
                    LAYER_NAMES[(self.layer - 1) as usize]
                ),
                false,
            );
            if n == 1 {
                self.count_knob1_press(now);
            }
            return;
        }
        let delta: i64 = if action == "right" { 1 } else { -1 };
        match n {
            1 => {
                let ids: Vec<&str> = st
                    .workspaces
                    .iter()
                    .map(|w| w.workspace_id.as_str())
                    .collect();
                if let Some(target) = step(&ids, st.workspace.as_deref().as_ref(), delta) {
                    if Some(target) != st.workspace.as_deref() {
                        herdr::call(
                            &self.sock_path,
                            "workspace.focus",
                            serde_json::json!({"workspace_id": target}),
                        );
                    }
                }
            }
            2 => {
                let ids: Vec<&str> = st.agents.iter().map(|s| s.as_str()).collect();
                if let Some(target) = step(&ids, st.active.as_deref().as_ref(), delta) {
                    if Some(target) != st.active.as_deref() {
                        herdr::call(
                            &self.sock_path,
                            "agent.focus",
                            serde_json::json!({"target": target}),
                        );
                    }
                }
            }
            3 => {
                let step_pct = if action == "right" {
                    BRIGHTNESS_STEP
                } else {
                    -BRIGHTNESS_STEP
                };
                let brightness = (self.brightness + step_pct).clamp(BRIGHTNESS_MIN, 100);
                if brightness != self.brightness {
                    self.brightness = brightness;
                    save_brightness(&self.brightness_file, brightness);
                    crate::log(&format!("brightness {brightness}%"), false);
                }
            }
            _ => {}
        }
    }

    /// Pressing knob 1 [`TRIPLE_PRESS`] times within [`TRIPLE_PRESS_WINDOW`]
    /// toggles all-workspaces mode.
    fn count_knob1_press(&mut self, now: f64) {
        self.knob1_presses
            .retain(|&t| now - t < TRIPLE_PRESS_WINDOW);
        self.knob1_presses.push(now);
        if self.knob1_presses.len() >= TRIPLE_PRESS {
            self.knob1_presses.clear();
            self.all_workspaces = !self.all_workspaces;
            crate::log(
                &format!(
                    "agent keys: {}",
                    if self.all_workspaces {
                        "all workspaces"
                    } else {
                        "focused workspace"
                    }
                ),
                false,
            );
        }
    }

    pub fn colors(&self, st: &State, now: f64) -> [(u8, u8, u8); 16] {
        let layer = LAYER_COLORS[(self.layer - 1) as usize];
        let flash_on = (now / FLASH) as i64 % 2 == 0;
        let agents = self.keyed_agents(st);
        let mut out = [OFF; 16];
        for (k, slot) in out.iter_mut().enumerate().take(keys::NUM_KEYS) {
            if k < AGENT_KEYS {
                let agent = agents.get(k).map(String::as_str);
                let status = agent.and_then(|a| st.status.get(a)).map(String::as_str);
                if agent.is_none() || (status == Some("blocked") && !flash_on) {
                    *slot = OFF;
                    continue;
                }
                let rgb = status_color(status.unwrap_or("idle"));
                *slot = if agent == st.active.as_deref() {
                    rgb
                } else {
                    (
                        (rgb.0 as u32 / INACTIVE_DIM) as u8,
                        (rgb.1 as u32 / INACTIVE_DIM) as u8,
                        (rgb.2 as u32 / INACTIVE_DIM) as u8,
                    )
                };
            } else {
                *slot = (
                    round_half_even(layer.0 as f64 * BOTTOM_BRIGHTNESS) as u8,
                    round_half_even(layer.1 as f64 * BOTTOM_BRIGHTNESS) as u8,
                    round_half_even(layer.2 as f64 * BOTTOM_BRIGHTNESS) as u8,
                );
            }
        }
        let scale = |c: u8| round_half_even(c as f64 * self.brightness as f64 / 100.0) as u8;
        out.map(|(r, g, b)| (scale(r), scale(g), scale(b)))
    }

    /// The pad's hardware event loop: fetch [`State`], show colours, wait
    /// for events, re-press, re-fetch, redraw.
    pub fn run(&mut self) -> Result<(), PadGone> {
        if self.pad.is_none() {
            return Err(PadGone("no pad".to_string()));
        }
        let start = Instant::now();
        let now = || start.elapsed().as_secs_f64();

        let mut st = State::fetch(&self.sock_path);
        let frame = to_frame(&self.colors(&st, now()));
        self.pad.as_mut().expect("checked above").show(&frame)?;

        let mut next_poll = now() + POLL;
        loop {
            let remaining = (next_poll - now()).max(0.0);
            let slots = self
                .pad
                .as_mut()
                .expect("checked above")
                .events(Duration::from_secs_f64(remaining))?;
            for &slot in &slots {
                let n = now();
                self.press(slot, &st, n);
                st = State::fetch(&self.sock_path);
            }
            if slots.is_empty() {
                st = State::fetch(&self.sock_path);
                next_poll = now() + POLL;
            }
            let frame = to_frame(&self.colors(&st, now()));
            self.pad.as_mut().expect("checked above").show(&frame)?;
        }
    }
}

fn to_frame(colors: &[(u8, u8, u8); 16]) -> [[u8; 3]; 16] {
    colors.map(|(r, g, b)| [r, g, b])
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
