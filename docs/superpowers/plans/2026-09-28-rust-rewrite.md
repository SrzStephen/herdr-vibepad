# Rust Rewrite of agentpad Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `agentpad` (a Linux daemon + two CLI tools that turn a USB keypad into a herdr controller) from Python to Rust, with no behavior change, working GitHub Actions CI, and a devcontainer for local development.

**Architecture:** One Cargo package with library modules (`hid`, `keys`, `led`, `herdr`, `daemon`) under `src/`, and four binaries under `src/bin/` that are thin wrappers around the library. Tests mirror the existing Python suite's approach exactly: real OS pipes for HID report decoding, a real `UnixListener` for the fake herdr server — no mocking framework.

**Tech Stack:** Rust (stable), `serde`/`serde_json` (herdr JSON-RPC), `nix`/`libc` (ioctl, poll, non-blocking I/O), `tempfile` (dev-dependency, test fixtures only).

**Spec:** `docs/superpowers/specs/2026-09-28-rust-rewrite-design.md`

## Global Constraints

- No Python left in the repo once Task 13 lands: `src/agentpad/`, `tests/*.py`, `tests/conftest.py`, `pyproject.toml`, `uv.lock`, `.venv/` are all deleted.
- One Cargo package, not a workspace. Library modules under `src/`; binaries under `src/bin/` (Cargo auto-discovers these — no `[[bin]]` sections needed).
- Runtime dependencies are exactly `serde`, `serde_json`, `nix`, `libc` — no CLI-parsing crate, no HID/evdev crate. `tempfile` is a dev-dependency only (test fixtures), not a runtime dependency.
- Every fallible protocol function in `hid`, `keys`, `led` returns `Result<_, String>` instead of exiting the process directly (the Python originals call `sys.exit`, which `daemon.py` relies on catching as `SystemExit` around the same functions — Rust has no equivalent, so this becomes explicit `Result` propagation). Only binary `main()` functions call `std::process::exit`.
- Each binary's `--help`/no-args usage text must be byte-identical to its Python module's docstring — `scripts/deb-smoke-test.sh` greps for exact phrases from it, and the spec requires no behavior change.
- Color/brightness math must reproduce Python's `round()` (round-half-to-even / banker's rounding) exactly via a shared `round_half_even` helper — `f64::round()` rounds half away from zero and will silently produce different LED colors at specific brightness values if used directly.
- Tests use real OS primitives (`nix::unistd::pipe`, `std::os::unix::net::UnixListener`) — no mock or trait abstraction layer over hardware or the herdr socket, matching the Python suite's approach.
- The `.deb` becomes `Architecture: amd64` (compiled binaries), not `all`.
- `diagrams` is dev-only tooling (regenerates `docs/*.svg`); it is never installed by the `.deb`, matching `scripts/diagrams.py`'s role today.
- Where a task ports an existing Python function, the exact algorithm and constants come from the named `src/agentpad/*.py` (or `scripts/diagrams.py`) file — that source is the spec for the algorithm; the plan gives the Rust signature and the adaptations, not a transcription of the logic.

## Review Focus

- Color-scaling math (dimming, brightness) must use round-half-to-even, not Rust's default round-half-away-from-zero, or specific brightness/dim combinations will render visibly different LED colors than the Python daemon did. Covered by a dedicated `round_half_even` test in Task 7.
- `step()`/`keyed_agents()` on an empty list (no agents in the focused workspace, or no workspaces at all — herdr just started, or a workspace was closed) must return "do nothing," not panic on an empty-list index. Covered by a test in Task 7.
- A HID protocol error while opening or preparing the pad (bad reply, wrong profile, disconnected mid-setup) must surface as a recoverable, retryable condition, not a panic that kills the daemon. This is hardware-dependent and can't be exercised without a real device — same gap the Python suite has — so it's enforced structurally in Task 6 (every fallible `keys::`/`led::`/`hid::` call in `Pad::open` uses `?`, never `.unwrap()`/`.expect()`) rather than by an automated test.
- A herdr reply that fails to parse as JSON, arrives with no data (closed connection), or carries a top-level `"error"` field must make the call return "no result" (logged once), not panic or crash the daemon. Covered by tests in Task 5.
- A hidraw key report shorter than 3 bytes must be ignored, not cause an out-of-bounds panic when the daemon inspects its usage bytes. Covered by a test in Task 6.

---

## Files

```
Cargo.toml
src/
  lib.rs       # module declarations + a shared `log(msg, once)` helper
  hid.rs       # shared hidraw find/send/request primitives (Task 3)
  keys.rs      # key-mapping protocol + parsing (Task 3)
  led.rs       # LED protocol (Task 4)
  herdr.rs     # herdr JSON-RPC client (Task 5)
  daemon.rs    # State, Pad, AgentPad (Tasks 6-7)
src/bin/
  side-keyboard-keys.rs   # Task 3
  side-keyboard-led.rs    # Task 4
  agentpad.rs             # Task 7
  diagrams.rs             # Task 8
tests/
  herdr.rs               # Task 5
  daemon_actions.rs       # Task 7
  support/mod.rs          # Task 5 (shared FakeHerdr test harness)
.devcontainer/devcontainer.json   # Task 12
```

### Task 1: Cargo project scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces: an empty `agentpad` lib crate that later tasks add modules to.

- [ ] **Step 1: Create `Cargo.toml`**

Package name `agentpad`, version `0.1.0` (matches `pyproject.toml` today), edition `2021`.

- [ ] **Step 2: Add dependencies**

Run: `cargo add serde --features derive`, `cargo add serde_json`, `cargo add nix --features fs,poll`, `cargo add libc`.

- [ ] **Step 3: Create `src/lib.rs`**

A top-of-file doc comment only (`//! agentpad: ...`). Module declarations (`pub mod hid;` etc.) are added by the tasks that create those files.

- [ ] **Step 4: Run `cargo build`**

Expected: success (empty lib compiles).

- [ ] **Step 5: Add `target/` to `.gitignore`**

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs .gitignore
git commit -m "Scaffold the Cargo project for the Rust rewrite"
```

### Task 2: Pre-commit config for Rust

**Files:**
- Modify: `.pre-commit-config.yaml`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Remove the ruff and uv-lock hooks**

Delete the `astral-sh/ruff-pre-commit` and `astral-sh/uv-pre-commit` repo blocks.

- [ ] **Step 2: Add local Rust lint hooks**

Add a `repo: local` block with two hooks, both `language: system`, `types: [rust]`, `pass_filenames: false`:
- `cargo-fmt`: `entry: cargo fmt --check`
- `cargo-clippy`: `entry: cargo clippy --all-targets -- -D warnings`

(No actively-maintained upstream pre-commit-rust repo exists to depend on instead — see spec.)

- [ ] **Step 3: Run `prek run --all-files`**

Expected: the hygiene hooks and shellcheck pass; `cargo-fmt`/`cargo-clippy` pass trivially against the empty lib from Task 1.

- [ ] **Step 4: Commit**

```bash
git add .pre-commit-config.yaml
git commit -m "Replace ruff/uv pre-commit hooks with cargo fmt/clippy"
```

### Task 3: hid + keys modules, side-keyboard-keys binary

**Files:**
- Create: `src/hid.rs`
- Create: `src/keys.rs`
- Create: `src/bin/side-keyboard-keys.rs`
- Modify: `src/lib.rs` (add `pub mod hid;` and `pub mod keys;`)

**Interfaces:**
- Consumes: nothing (first module with real logic).
- Produces (used by Task 4's `led.rs`, Task 6/7's `daemon.rs`):
  ```rust
  // hid.rs
  pub fn find_hidraw(vendor_product: &str, input_suffix: &str) -> Option<PathBuf>;
  pub fn find_input_event_nodes(vendor_hex: &str, product_hex: &str) -> Vec<PathBuf>;
  pub fn send(fd: RawFd, payload: &[u8]) -> io::Result<()>;
  pub fn request(fd: RawFd, payload: &[u8], is_reply: impl Fn(&[u8]) -> bool, what: &str) -> Result<Vec<u8>, String>;

  // keys.rs
  pub const NUM_SLOTS: usize = 25;
  pub const NUM_KEYS: usize = 16;
  pub const KNOB_PARTS: [&str; 3] = ["press", "right", "left"];
  pub const TYPE_KEY: u8 = 0x20;
  pub const TYPE_CONSUMER: u8 = 0x30;
  pub const TYPE_DISABLED: u8 = 0x13;
  pub const TYPE_MACRO: u8 = 0x60;
  pub const TYPE_FUNCTION: u8 = 0x1F;
  pub fn parse_key(text: &str) -> Result<[u8; 4], String>;
  pub fn describe(entry: [u8; 4]) -> String;
  pub fn parse_slot(text: &str) -> Result<usize, String>;
  pub fn slot_name(slot: usize) -> String;
  pub fn find_device() -> Result<PathBuf, String>;
  pub fn get_profile(fd: RawFd) -> Result<(u8, u8), String>;      // (active, count)
  pub fn select_profile(fd: RawFd, n: u8) -> Result<(), String>;
  pub fn read_table(fd: RawFd) -> Result<Vec<[u8; 4]>, String>;
  pub fn write_slot(fd: RawFd, slot: usize, entry: [u8; 4]) -> Result<(), String>;
  pub fn save_backup(table: &[[u8; 4]], profile: u8) -> io::Result<()>;
  pub fn backup_path(profile: u8) -> PathBuf;
  ```

- [ ] **Step 1: Write the failing tests, ported from `tests/test_keys.py`**

```rust
#[test]
fn parses_plain_letter() { assert_eq!(keys::parse_key("a").unwrap(), [keys::TYPE_KEY, 0, 0x04, 0]); }

#[test]
fn parses_modifiers() { assert_eq!(keys::parse_key("ctrl+shift+t").unwrap(), [keys::TYPE_KEY, 0x03, 0x17, 0]); }

#[test]
fn parses_f13_and_f24() {
    assert_eq!(keys::parse_key("f13").unwrap(), [keys::TYPE_KEY, 0, 0x68, 0]);
    assert_eq!(keys::parse_key("f24").unwrap(), [keys::TYPE_KEY, 0, 0x73, 0]);
}

#[test]
fn parses_consumer_and_function_keys() {
    assert_eq!(keys::parse_key("volup").unwrap(), [keys::TYPE_CONSUMER, 0xE9, 0, 0]);
    assert_eq!(keys::parse_key("profileswitch").unwrap(), [keys::TYPE_FUNCTION, 0x13, 0, 0]);
}

#[test]
fn describe_round_trips() {
    for text in ["a", "ctrl+shift+t", "shift+f18", "esc", "mute", "profileswitch"] {
        assert_eq!(keys::describe(keys::parse_key(text).unwrap()), text);
    }
}

#[test]
fn parses_and_names_slots() {
    for (text, slot) in [("0", 0), ("15", 15), ("knob1.press", 16), ("knob3.left", 24)] {
        assert_eq!(keys::parse_slot(text).unwrap(), slot);
        assert_eq!(keys::slot_name(slot), if text.chars().all(|c| c.is_ascii_digit()) {
            format!("key {text}")
        } else {
            text.to_string()
        });
    }
}

#[test]
fn bad_key_is_an_error() {
    assert!(keys::parse_key("hyper+q").is_err());
}
```

Place these in an inline `#[cfg(test)] mod tests` at the bottom of `src/keys.rs`.

- [ ] **Step 2: Run `cargo test`**

Expected: FAIL (module doesn't exist yet).

- [ ] **Step 3: Implement `src/keys.rs`'s pure functions**

Port the `MODIFIERS`/`USAGES`/`CONSUMER`/`FUNCTIONS` tables and `parse_key`/`describe`/`parse_slot`/`slot_name` from `src/agentpad/keys.py` — keep every numeric value exactly as in the Python source (they're fixed by the device's protocol). Where Python calls `sys.exit(msg)`, return `Err(msg.to_string())` instead.

- [ ] **Step 4: Run `cargo test`**

Expected: PASS.

- [ ] **Step 5: Implement `src/hid.rs`**

Port `find_device`/`send`/`request` from `src/agentpad/keys.py`, folding in `led.py`'s near-identical copies and `daemon.py`'s `report_node`/`input_nodes` directory-scanning shape (see Task 6) into the two generic functions above. No dedicated tests — matches the Python original, which has none for this hardware I/O either.

- [ ] **Step 6: Implement the rest of `src/keys.rs`**

Port `find_device`/`get_profile`/`select_profile`/`read_table`/`write_slot`/`save_backup`/`backup_path` from `src/agentpad/keys.py`, calling into `hid::find_hidraw`/`hid::send`/`hid::request`. No new tests (matches Python parity).

- [ ] **Step 7: Run `cargo build`**

Expected: success.

- [ ] **Step 8: Implement `src/bin/side-keyboard-keys.rs`**

Port `keys.py`'s `main()`/`run()` (the `read`/`set`/`all`/`restore`/`profile` subcommands and the `--profile` flag). The usage text printed with no args or `-h`/`--help` must be byte-identical to `keys.py`'s module docstring. On a `Result::Err` from any library call, print the message to stderr and `std::process::exit(1)`.

- [ ] **Step 9: Run `cargo build --bin side-keyboard-keys` then `./target/debug/side-keyboard-keys`**

Expected: prints the usage text and exits nonzero (no args).

- [ ] **Step 10: Commit**

```bash
git add src/hid.rs src/keys.rs src/bin/side-keyboard-keys.rs src/lib.rs
git commit -m "Port keys.py to hid.rs/keys.rs and the side-keyboard-keys binary"
```

### Task 4: led module, side-keyboard-led binary

**Files:**
- Create: `src/led.rs`
- Create: `src/bin/side-keyboard-led.rs`
- Modify: `src/lib.rs` (add `pub mod led;`)

**Interfaces:**
- Consumes: `crate::hid::{find_hidraw, send}` (Task 3), `crate::keys::NUM_KEYS` (Task 3 — reused, not redefined, removing the duplication that exists between `keys.py` and `led.py` today).
- Produces (used by Task 6/7's `daemon.rs`):
  ```rust
  pub const CUSTOM: u8 = 5;
  pub fn find_device() -> Result<PathBuf, String>;
  pub fn read_state(fd: RawFd) -> Result<[u8; 11], String>;   // s[0..11], mirrors Python's r[5:16]
  pub fn write_state(fd: RawFd, s: &mut [u8; 11]);
  pub fn set_key(fd: RawFd, index: usize, rgb: [u8; 3]) -> io::Result<()>;
  pub fn set_all_keys(fd: RawFd, rgb: [u8; 3]) -> io::Result<()>;
  pub fn ensure_custom(fd: RawFd, cur: &[u8; 11]) -> io::Result<()>;
  pub fn parse_rgb(hexstr: &str) -> Result<[u8; 3], String>;
  ```

- [ ] **Step 1: Implement `src/led.rs`**

Port `src/agentpad/led.py` in full. No dedicated unit tests exist for this module in the Python suite either — this task is verified by build success and the manual `--help` check below, not TDD.

- [ ] **Step 2: Run `cargo build`**

Expected: success.

- [ ] **Step 3: Implement `src/bin/side-keyboard-led.rs`**

Port `led.py`'s `main()` (the `read`/`raw`/`set`/`key`/`flash` subcommands). Usage text byte-identical to `led.py`'s module docstring.

- [ ] **Step 4: Run `cargo build --bin side-keyboard-led` then `./target/debug/side-keyboard-led`**

Expected: prints the usage text and exits nonzero (no args).

- [ ] **Step 5: Commit**

```bash
git add src/led.rs src/bin/side-keyboard-led.rs src/lib.rs
git commit -m "Port led.py to led.rs and the side-keyboard-led binary"
```

### Task 5: herdr JSON-RPC client

**Files:**
- Create: `src/herdr.rs`
- Create: `tests/herdr.rs`
- Create: `tests/support/mod.rs`
- Modify: `src/lib.rs` (add `pub mod herdr;` and a shared `pub fn log(msg: &str, once: bool)`)
- Modify: `Cargo.toml` (dev-dependency)

**Interfaces:**
- Consumes: nothing new.
- Produces (used by Task 6/7's `daemon.rs`, and by `tests/support`):
  ```rust
  // lib.rs
  pub fn log(msg: &str, once: bool);   // prints via println! (stdout, like Python's log()); "once" dedupes an immediately-repeated message

  // herdr.rs
  pub fn call(sock_path: &Path, method: &str, params: serde_json::Value) -> Option<serde_json::Value>;

  // tests/support/mod.rs
  pub struct FakeHerdr { pub sock_path: PathBuf, /* private: tempdir, join handle, call log */ }
  impl FakeHerdr {
      pub fn start<F>(handler: F) -> FakeHerdr
      where F: Fn(&str, serde_json::Value) -> Result<serde_json::Value, serde_json::Value> + Send + Sync + 'static;
      pub fn calls(&self) -> Vec<(String, serde_json::Value)>;
  }
  ```

- [ ] **Step 1: Add the `tempfile` dev-dependency**

Run: `cargo add tempfile --dev`.

- [ ] **Step 2: Implement `tests/support/mod.rs`'s `FakeHerdr`**

A `std::os::unix::net::UnixListener` bound inside a `tempfile::TempDir` (short path — Unix socket paths max out at 108 bytes, same constraint noted in `tests/conftest.py`), served on a background thread. Per connection: read one newline-delimited JSON request `{"id", "method", "params"}`, record `(method, params)`, call `handler(method, params)`, write back `{"id", "result": ...}` on `Ok` or `{"id", "error": ...}` on `Err`, followed by `\n`.

- [ ] **Step 3: Write the failing tests in `tests/herdr.rs`**

```rust
mod support;
use std::path::Path;

#[test]
fn returns_the_result_field_on_success() {
    let fake = support::FakeHerdr::start(|_method, _params| Ok(serde_json::json!({"ok": true})));
    let result = agentpad::herdr::call(&fake.sock_path, "ping", serde_json::json!({}));
    assert_eq!(result, Some(serde_json::json!({"ok": true})));
}

#[test]
fn returns_none_on_an_error_reply() {
    let fake = support::FakeHerdr::start(|_method, _params| Err(serde_json::json!({"message": "boom"})));
    assert_eq!(agentpad::herdr::call(&fake.sock_path, "ping", serde_json::json!({})), None);
}

#[test]
fn returns_none_when_nothing_is_listening() {
    let result = agentpad::herdr::call(Path::new("/nonexistent/herdr.sock"), "ping", serde_json::json!({}));
    assert_eq!(result, None);
}
```

- [ ] **Step 4: Run `cargo test --test herdr`**

Expected: FAIL (`herdr::call` doesn't exist).

- [ ] **Step 5: Implement `log` in `src/lib.rs`**

A `static LAST_LOG: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);`, porting `daemon.py`'s `log()`: print the message with `println!` (stdout — the systemd service's log goes through stdout, same as Python's `print(..., flush=True)`); if `once` and the message equals the last one printed, skip it; otherwise print and remember it.

- [ ] **Step 6: Implement `src/herdr.rs`'s `call`**

Port `daemon.py`'s `herdr()` function: connect a `UnixStream` to `sock_path` with a 2-second read and write timeout, send `{"id": "agentpad", "method": method, "params": params}\n`, read the reply with `std::io::BufReader::new(stream).read_until(b'\n', &mut buf)`, parse it as JSON. On any connect/timeout/parse error, call `crate::log(&format!("herdr {method}: {e}"), true)` and return `None`. If the parsed reply has an `"error"` field, call `crate::log(&format!("herdr {method}: {error}"), false)` and return `None`. Otherwise return the `"result"` field.

- [ ] **Step 7: Run `cargo test --test herdr`**

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/herdr.rs src/lib.rs tests/herdr.rs tests/support/mod.rs
git commit -m "Port daemon.py's herdr() to herdr.rs, with a real-socket test harness"
```

### Task 6: daemon core — State, position, Pad event loop

**Files:**
- Create: `src/daemon.rs`
- Modify: `src/lib.rs` (add `pub mod daemon;`)

**Interfaces:**
- Consumes: `crate::hid::{find_hidraw, find_input_event_nodes, send}` (Task 3), `crate::keys::{NUM_KEYS, NUM_SLOTS, KNOB_PARTS, parse_key, get_profile, select_profile, read_table, write_slot}` (Task 3), `crate::led::{read_state, write_state, CUSTOM}` (Task 4), `crate::herdr::call` (Task 5), `crate::log` (Task 5).
- Produces (used by Task 7 and `src/bin/diagrams.rs` in Task 8):
  ```rust
  pub const PAD_PROFILE: u8 = 5;
  pub const LED_HW_BRIGHTNESS: u8 = 4;
  pub fn position(i: usize) -> usize;

  pub struct Workspace { pub workspace_id: String, pub number: u32 }
  pub struct State {
      pub workspaces: Vec<Workspace>,
      pub workspace: Option<String>,
      pub all_agents: Vec<String>,
      pub agents: Vec<String>,
      pub status: std::collections::HashMap<String, String>,
      pub active: Option<String>,
  }
  impl State { pub fn fetch(sock_path: &Path) -> State; }

  #[derive(Debug)]
  pub struct PadGone(pub String);
  impl From<String> for PadGone { fn from(s: String) -> Self { PadGone(s) } }

  pub struct Pad { /* fields private: hid, reports, inputs, down, frame */ }
  impl Pad {
      pub fn open() -> Result<Pad, PadGone>;
      pub fn events(&mut self, timeout: std::time::Duration) -> Result<Vec<usize>, PadGone>;
      pub fn show(&mut self, colors: &[[u8; 3]; 16]);
  }
  impl Drop for Pad { fn drop(&mut self); }   // closes hid, reports, and every inputs fd
  ```

  `Pad::open`/`events`/`show` replace Python's `Pad.__init__`/`prepare`/`events`/`show`/`drain`/`close`; using `impl Drop` instead of an explicit `close()` method is the one deliberate simplification here — Rust gives fd cleanup on every exit path for free.

- [ ] **Step 1: Write the failing tests for `position()`, ported from `tests/test_pad.py`**

```rust
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
}
```

- [ ] **Step 2: Run `cargo test`**

Expected: FAIL (`position` doesn't exist).

- [ ] **Step 3: Implement `position(i: usize) -> usize`**

Port the formula from `daemon.py`'s `position()`: `(3 - i % 4) * 4 + i / 4` (integer division).

- [ ] **Step 4: Run `cargo test`**

Expected: PASS.

- [ ] **Step 5: Write a failing test for the slot/code table**

```rust
#[test]
fn every_pad_slot_has_a_distinct_code() {
    assert_eq!(code_to_slot().len(), keys::NUM_SLOTS);
}
```

- [ ] **Step 6: Implement the private slot/code table**

Port `daemon.py`'s `SLOT_KEYS` (`["f13".."f24"] + ["shift+f13".."shift+f24"] + ["ctrl+f13"]`, 25 entries, each parsed with `keys::parse_key`) and `CODE_TO_SLOT` as a private `fn code_to_slot() -> &'static HashMap<(u8, u8), usize>` built once behind a `std::sync::OnceLock`.

- [ ] **Step 7: Run `cargo test`**

Expected: PASS.

- [ ] **Step 8: Write the failing tests for `Pad::events`, ported from `tests/test_pad.py`'s `feed()`**

```rust
fn feed(reports: &[&str]) -> Vec<usize> {
    let (hid_r, _hid_w) = nix::unistd::pipe().unwrap();
    let (rep_r, rep_w) = nix::unistd::pipe().unwrap();
    // set rep_r non-blocking, like the real hidraw node (os.set_blocking(rep_r, False) in Python)
    let mut pad = Pad { hid: hid_r, reports: rep_r, inputs: vec![], down: vec![], frame: None };
    let mut slots = vec![];
    for report in reports {
        let bytes: Vec<u8> = report.split_whitespace().flat_map(|pair| {
            (0..pair.len()).step_by(2).map(|i| u8::from_str_radix(&pair[i..i + 2], 16).unwrap())
        }).collect();
        nix::unistd::write(&rep_w, &bytes).unwrap();
        slots.extend(pad.events(std::time::Duration::ZERO).unwrap());
    }
    slots
}

#[test]
fn decodes_reports_captured_from_the_pad() {
    let reports = [
        "0200 6c00 0000 0000", "0000 0000 0000 0000", "0200 6f00 0000 0000",
        "0200 6f72 0000 0000", "0000 6f72 0000 0000", "0000 0000 0000 0000",
        "0200 6d00 0000 0000", "0000 0000 0000 0000", "0100 6800 0000 0000",
        "0000 0000 0000 0000", "0000 6b00 0000 0000", "0000 0000 0000 0000",
        "0200 6800 0000 0000",
    ];
    assert_eq!(feed(&reports), vec![16, 19, 22, 17, 24, 3, 12]);
}

#[test]
fn a_held_key_counts_once() {
    assert_eq!(feed(&["0000 6800 0000 0000", "0000 6800 0000 0000", "0000 6800 0000 0000"]), vec![0]);
}

#[test]
fn a_short_report_is_ignored_not_a_panic() {
    assert_eq!(feed(&["0000", "0000 6800 0000 0000"]), vec![0]);
}
```

Each hex pair is two ASCII hex digits separated from its neighbors by whitespace only for readability; decode by stripping whitespace and reading two-hex-digit chunks (mirrors Python's `bytes.fromhex`).

- [ ] **Step 9: Run `cargo test`**

Expected: FAIL (`Pad`/`events` don't exist).

- [ ] **Step 10: Implement `Pad`'s private fields and `Pad::events`**

Port `daemon.py`'s `Pad.events()`: `nix::poll::poll` the `hid`/`reports`/`inputs` fds for `timeout`; for each readable fd, `nix::unistd::read` up to 4096 bytes. A read returning `Err(Errno::EAGAIN | Errno::EWOULDBLOCK)` means no data — treat it as "no data" and move on to the next fd, not an error (mirrors Python's `except BlockingIOError: continue`). Skip frames from any fd other than `reports`, and skip a `reports` frame shorter than 3 bytes (mirrors `if fd != self.reports or len(data) < 3: continue`). Decode the usage bytes (`data[2..8]`, skipping zeros) against `self.down` and `code_to_slot()`, updating `self.down`.

- [ ] **Step 11: Run `cargo test`**

Expected: PASS.

- [ ] **Step 12: Implement `Pad::open`**

Port `daemon.py`'s `Pad.__init__`/`Pad.prepare`/`report_node`/`input_nodes`: open the hidraw report node (`hid::find_hidraw("00006D7D:0000DCFC", "/input1\n")`) and the config node (`keys::find_device()`), grab each of `hid::find_input_event_nodes("6d7d", "dcfc")` with `libc::ioctl(fd, 0x40044590 as libc::c_ulong, 1)` (`EVIOCGRAB`; a negative return is non-fatal, matching Python, which doesn't check it), then select `PAD_PROFILE`, rewrite any wrong slot via `keys::write_slot` (backing up first with `keys::save_backup`), and set the LEDs to per-key custom mode at `LED_HW_BRIGHTNESS` via `led::read_state`/`led::write_state`. Every fallible `keys::`/`led::`/`hid::` call must use `?` (via the `From<String> for PadGone` impl above) — never `.unwrap()`/`.expect()` — so a HID protocol error becomes a retryable `PadGone`, not a crash (see Review Focus). No new automated test — this needs real hardware, matching the Python original, which is untested here too; verify with `cargo build`.

- [ ] **Step 13: Implement `Pad::show` and `impl Drop for Pad`**

Port `daemon.py`'s `Pad.show`/`Pad.drain`/`Pad.close`: `show` remaps `colors` (indexed by physical position) into pad slot order via `position(i)` and sends one 48-byte per-key frame via `hid::send`, skipping the write if the frame is unchanged from `self.frame`. `Drop` closes `hid`, `reports`, and every fd in `inputs`.

- [ ] **Step 14: Implement `State`**

Port `daemon.py`'s `State.__init__`/`id_number`: call `herdr::call(sock_path, "workspace.list", ...)` and `herdr::call(sock_path, "agent.list", ...)`, sort and derive `workspaces`/`workspace`/`all_agents`/`agents`/`status`/`active` exactly as the Python does. No new test yet — Task 7 exercises this through `AgentPad`.

- [ ] **Step 15: Run `cargo build` then `cargo test`**

Expected: build succeeds; all tests still pass.

- [ ] **Step 16: Add `pub mod daemon;` to `src/lib.rs`**

- [ ] **Step 17: Commit**

```bash
git add src/daemon.rs src/lib.rs
git commit -m "Port daemon.py's State and Pad to daemon.rs"
```

### Task 7: AgentPad behavior, brightness, colors, agentpad binary

**Files:**
- Modify: `src/daemon.rs`
- Create: `src/bin/agentpad.rs`
- Create: `tests/daemon_actions.rs`
- Modify: `tests/support/mod.rs` (add a two-workspace herdr fixture)

**Interfaces:**
- Consumes: `crate::daemon::{State, Workspace, Pad, PadGone, position}` (Task 6), `crate::keys::{NUM_KEYS, KNOB_PARTS}` (Task 3), `crate::herdr::call` (Task 5), `tests/support::FakeHerdr` (Task 5).
- Produces (used by `src/bin/diagrams.rs` in Task 8):
  ```rust
  pub const LAYER_COLORS: [(u8, u8, u8); 3] = [(255, 50, 0), (0, 60, 255), (150, 0, 255)];
  pub const LAYER_NAMES: [&str; 3] = ["Claude", "Codex", "Kiro"];
  pub const BOTTOM_KEYS: [[Option<&str>; 4]; 3] = [
      [Some("1"), Some("2"), Some("3"), Some("esc")],
      [Some("1"), Some("2"), Some("3"), Some("esc")],
      [Some("y"), Some("n"), Some("t"), Some("esc")],
  ];
  pub const BOTTOM_BRIGHTNESS: f64 = 0.2;
  pub const OFF: (u8, u8, u8) = (0, 0, 0);
  pub const INACTIVE_DIM: u32 = 5;
  pub const FLASH: f64 = 0.5;
  pub const AGENT_KEYS: usize = 12;
  pub const BRIGHTNESS_STEP: i32 = 5;
  pub const BRIGHTNESS_MIN: i32 = 5;
  pub const TRIPLE_PRESS: usize = 3;
  pub const TRIPLE_PRESS_WINDOW: f64 = 1.0;
  pub fn status_color(status: &str) -> (u8, u8, u8);   // "working"/"blocked"/"done"/"idle", else idle
  pub fn round_half_even(x: f64) -> i64;

  pub struct AgentPad {
      pub layer: u8,
      pub brightness: i32,
      pub all_workspaces: bool,
      pub knob1_presses: Vec<f64>,
      // private: pad: Option<Pad>, sock_path: PathBuf, brightness_file: PathBuf
  }
  impl AgentPad {
      pub fn new(pad: Option<Pad>, sock_path: PathBuf, brightness_file: PathBuf) -> AgentPad;
      pub fn press(&mut self, slot: usize, st: &State, now: f64);
      pub fn colors(&self, st: &State, now: f64) -> [[u8; 3]; 16];
      pub fn run(&mut self) -> Result<(), PadGone>;
  }
  ```

  `press`/`colors` take `now: f64` (monotonic seconds) explicitly rather than reading a clock internally — Python monkeypatches `time.monotonic` in tests; Rust has no equivalent, so the clock value is threaded through as a parameter instead. `run()`'s hardware loop sources `now` from a real monotonic clock at each iteration.

- [ ] **Step 1: Add the two-workspace herdr fixture to `tests/support/mod.rs`**

Port `tests/conftest.py`'s `FakeHerdr` class (panes `w1: [p1, p2, p3]`, `w2: [p1]`, `workspace.list`/`agent.list`/`workspace.focus`/`agent.focus`/`pane.send_keys` handling, call recording) as `pub fn two_workspace_herdr() -> support::FakeHerdr`, built on `FakeHerdr::start` from Task 5.

- [ ] **Step 2: Write the failing tests in `tests/daemon_actions.rs`**

```rust
mod support;
use agentpad::daemon::{self, AgentPad, State};
use agentpad::keys;
use std::path::Path;

fn knob(n: usize, part: &str) -> usize {
    keys::NUM_KEYS + 3 * (n - 1) + keys::KNOB_PARTS.iter().position(|p| *p == part).unwrap()
}
fn key_at(pos: usize) -> usize {
    (0..keys::NUM_KEYS).find(|&i| daemon::position(i) == pos).unwrap()
}
fn press(pad: &mut AgentPad, slot: usize, sock: &Path, now: f64) -> State {
    pad.press(slot, &State::fetch(sock), now);
    State::fetch(sock)
}

#[test]
fn knob2_steps_through_agents_and_wraps() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    let mut active = vec![];
    for _ in 0..3 { active.push(press(&mut pad, knob(2, "right"), &fake.sock_path, 0.0).active); }
    assert_eq!(active, [Some("w1:p2".into()), Some("w1:p3".into()), Some("w1:p1".into())]);
    assert_eq!(press(&mut pad, knob(2, "left"), &fake.sock_path, 0.0).active, Some("w1:p3".into()));
}

#[test]
fn knob1_steps_through_workspaces_and_wraps() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    assert_eq!(press(&mut pad, knob(1, "right"), &fake.sock_path, 0.0).workspace, Some("w2".into()));
    assert_eq!(press(&mut pad, knob(1, "right"), &fake.sock_path, 0.0).workspace, Some("w1".into()));
    assert_eq!(press(&mut pad, knob(1, "left"), &fake.sock_path, 0.0).workspace, Some("w2".into()));
}

#[test]
fn agent_key_focuses_that_agent() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    assert_eq!(press(&mut pad, key_at(2), &fake.sock_path, 0.0).active, Some("w1:p3".into()));
    assert_eq!(press(&mut pad, key_at(0), &fake.sock_path, 0.0).active, Some("w1:p1".into()));
}

#[test]
fn agent_key_without_agent_does_nothing() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    press(&mut pad, key_at(7), &fake.sock_path, 0.0);
    assert!(!fake.calls().iter().any(|(m, _)| m == "agent.focus"));
}

#[test]
fn knob_press_selects_layer() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    for n in [3, 2, 1] {
        press(&mut pad, knob(n, "press"), &fake.sock_path, 0.0);
        assert_eq!(pad.layer, n as u8);
    }
}

#[test]
fn bottom_row_sends_layer_keys_to_active_agent() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    for layer in 1..=3usize {
        press(&mut pad, knob(layer, "press"), &fake.sock_path, 0.0);
        for pos in 12..16 { press(&mut pad, key_at(pos), &fake.sock_path, 0.0); }
    }
    let sent: Vec<(String, String)> = fake.calls().into_iter()
        .filter(|(m, _)| m == "pane.send_keys")
        .map(|(_, p)| (p["pane_id"].as_str().unwrap().to_string(), p["keys"][0].as_str().unwrap().to_string()))
        .collect();
    let expected: Vec<&str> = [1, 2, 3].iter().flat_map(|&l| daemon::BOTTOM_KEYS[l - 1].iter().flatten().copied()).collect();
    assert_eq!(expected[..4], ["1", "2", "3", "esc"]);
    assert_eq!(sent, expected.iter().map(|k| ("w1:p1".to_string(), k.to_string())).collect::<Vec<_>>());
}

#[test]
fn knob3_changes_brightness_within_limits_and_remembers() {
    let fake = support::two_workspace_herdr();
    let brightness_file = tempfile::NamedTempFile::new().unwrap().path().to_path_buf();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), brightness_file.clone());
    assert_eq!(pad.brightness, 100);
    press(&mut pad, knob(3, "right"), &fake.sock_path, 0.0);
    assert_eq!(pad.brightness, 100);
    for _ in 0..40 { press(&mut pad, knob(3, "left"), &fake.sock_path, 0.0); }
    assert_eq!(pad.brightness, daemon::BRIGHTNESS_MIN);
    assert_eq!(pad.brightness, 5);
    press(&mut pad, knob(3, "right"), &fake.sock_path, 0.0);
    assert_eq!(pad.brightness, 10);
    assert_eq!(std::fs::read_to_string(&brightness_file).unwrap().trim(), "10");
    assert_eq!(AgentPad::new(None, fake.sock_path.clone(), brightness_file).brightness, 10);
}

#[test]
fn no_herdr_means_empty_state() {
    let st = State::fetch(Path::new("/nonexistent/herdr.sock"));
    assert!(st.workspaces.is_empty() && st.agents.is_empty() && st.active.is_none());
}

fn triple_press_knob1(pad: &mut AgentPad, sock: &Path, gap: f64, start: f64) {
    for i in 0..3 { press(pad, knob(1, "press"), sock, start + i as f64 * gap); }
}

#[test]
fn three_quick_knob1_presses_toggle_all_workspaces() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 100.0);
    assert!(pad.all_workspaces && pad.layer == 1);
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 200.0);
    assert!(!pad.all_workspaces);
}

#[test]
fn slow_or_double_knob1_presses_do_not_toggle() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    triple_press_knob1(&mut pad, &fake.sock_path, 0.6, 300.0);   // 1.2s from first to third
    assert!(!pad.all_workspaces);
    pad.knob1_presses.clear();
    press(&mut pad, knob(1, "press"), &fake.sock_path, 400.0);
    press(&mut pad, knob(1, "press"), &fake.sock_path, 400.1);
    assert!(!pad.all_workspaces);
}

#[test]
fn all_workspaces_key_jumps_to_agent_in_other_workspace() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    assert_eq!(press(&mut pad, key_at(3), &fake.sock_path, 0.0).active, Some("w1:p1".into()));
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 500.0);
    let st = press(&mut pad, key_at(3), &fake.sock_path, 0.0);
    assert_eq!((st.workspace, st.active), (Some("w2".into()), Some("w2:p1".into())));
    assert_eq!(press(&mut pad, key_at(0), &fake.sock_path, 0.0).active, Some("w1:p1".into()));
}

#[test]
fn all_workspaces_mode_lights_every_agent() {
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    let lit = |pad: &AgentPad, st: &State| pad.colors(st, 0.0)[..12].iter().filter(|&&c| c != daemon::OFF).count();
    assert_eq!(lit(&pad, &State::fetch(&fake.sock_path)), 3);
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 600.0);
    assert_eq!(lit(&pad, &State::fetch(&fake.sock_path)), 4);
}
```

Also, ported from `tests/test_leds.py` (same file):

```rust
fn state_with(agents: &[&str], active: Option<&str>, status: &[(&str, &str)]) -> State {
    State {
        workspaces: vec![], workspace: None,
        all_agents: agents.iter().map(|s| s.to_string()).collect(),
        agents: agents.iter().map(|s| s.to_string()).collect(),
        status: status.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        active: active.map(str::to_string),
    }
}

#[test]
fn bottom_row_is_layer_colour_at_20_percent() {
    for layer in 1..=3u8 {
        let pad = AgentPad::new(None, std::env::temp_dir().join("unused"), std::env::temp_dir().join("unused"));
        let mut pad = pad; pad.layer = layer;
        let (r, g, b) = daemon::LAYER_COLORS[(layer - 1) as usize];
        let expected = [daemon::round_half_even(r as f64 * 0.2) as u8,
                         daemon::round_half_even(g as f64 * 0.2) as u8,
                         daemon::round_half_even(b as f64 * 0.2) as u8];
        for i in 12..16 {
            assert_eq!(pad.colors(&state_with(&[], None, &[]), 0.0)[i], (expected[0], expected[1], expected[2]));
        }
    }
}

#[test]
fn agent_keys_show_status_active_bright_others_dimmed() {
    let pad = AgentPad::new(None, std::env::temp_dir().join("unused"), std::env::temp_dir().join("unused"));
    let st = state_with(&["a", "b", "c", "d"], Some("b"),
        &[("a", "working"), ("b", "done"), ("c", "idle"), ("d", "unknown")]);
    let out = pad.colors(&st, 0.0);
    let dim = |c: (u8, u8, u8)| ((c.0 as u32 / daemon::INACTIVE_DIM) as u8, (c.1 as u32 / daemon::INACTIVE_DIM) as u8, (c.2 as u32 / daemon::INACTIVE_DIM) as u8);
    assert_eq!(out[0], dim(daemon::status_color("working")));
    assert_eq!(out[1], daemon::status_color("done"));
    assert_eq!(out[2], dim(daemon::status_color("idle")));
    assert_eq!(out[3], out[2]);   // unknown shows as idle
    for i in 4..12 { assert_eq!(out[i], daemon::OFF); }
}

#[test]
fn blocked_agent_flashes() {
    let pad = AgentPad::new(None, std::env::temp_dir().join("unused"), std::env::temp_dir().join("unused"));
    let st = state_with(&["a"], Some("a"), &[("a", "blocked")]);
    assert_eq!(pad.colors(&st, 0.1)[0], daemon::status_color("blocked"));
    assert_eq!(pad.colors(&st, daemon::FLASH + 0.1)[0], daemon::OFF);
}

#[test]
fn brightness_scales_everything() {
    let mut pad = AgentPad::new(None, std::env::temp_dir().join("unused"), std::env::temp_dir().join("unused"));
    let st = state_with(&["a"], Some("a"), &[]);
    pad.brightness = 100;
    let full = pad.colors(&st, 0.0);
    pad.brightness = 50;
    let half = pad.colors(&st, 0.0);
    for i in 0..16 {
        let expect = (
            daemon::round_half_even(full[i].0 as f64 / 2.0) as u8,
            daemon::round_half_even(full[i].1 as f64 / 2.0) as u8,
            daemon::round_half_even(full[i].2 as f64 / 2.0) as u8,
        );
        assert_eq!(half[i], expect);
    }
}
```

And a new test, not in the Python suite, pinning the rounding convention itself (Review Focus):

```rust
#[test]
fn round_half_even_matches_pythons_banker_rounding() {
    for (input, expected) in [(0.5, 0), (1.5, 2), (2.5, 2), (3.5, 4), (25.5, 26), (127.5, 128)] {
        assert_eq!(daemon::round_half_even(input), expected);
    }
}
```

And a new test for the empty-list Review Focus item:

```rust
#[test]
fn knob_turn_does_nothing_with_no_agents_or_workspaces() {
    let fake = support::FakeHerdr::start(|method, _| Ok(match method {
        "workspace.list" => serde_json::json!({"workspaces": []}),
        "agent.list" => serde_json::json!({"agents": []}),
        _ => serde_json::json!({}),
    }));
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), std::env::temp_dir().join("unused"));
    press(&mut pad, knob(1, "right"), &fake.sock_path, 0.0);   // must not panic
    press(&mut pad, knob(2, "right"), &fake.sock_path, 0.0);   // must not panic
}
```

- [ ] **Step 3: Run `cargo test`**

Expected: FAIL (`AgentPad` and friends don't exist / don't compile).

- [ ] **Step 4: Implement `round_half_even`**

`(x - x.floor() == 0.5) then (round down to the even neighbor), else x.round()` — i.e. only the exact-half case needs special handling; every other value matches `f64::round()`. (Python's `round()` is round-half-to-even in general, but for the byte-valued inputs this codebase ever rounds, exact ties are the only place it diverges from `f64::round()`.)

- [ ] **Step 5: Implement `status_color`, `step`, brightness load/save, and `AgentPad`**

Port `daemon.py`'s `STATUS_COLORS` lookup (as `status_color`), `step()` (returning `None` for an empty slice — Review Focus), `load_brightness`/`save_brightness` (parameterized by path instead of reading a module constant), and `AgentPad`'s `keyed_agents`/`press`/`key`/`knob`/`count_knob1_press`/`colors` methods. `colors()` and `count_knob1_press()` take the `now: f64` parameter in place of Python's `time.monotonic()` calls.

- [ ] **Step 6: Run `cargo test`**

Expected: PASS.

- [ ] **Step 7: Implement `AgentPad::run` and `src/bin/agentpad.rs`**

Port `daemon.py`'s `AgentPad.run()` (the hardware event loop: fetch `State`, show colors, wait for events, re-press, re-fetch, redraw) and `main()` (the `Pad::open` retry-with-2s-sleep loop, reading `AGENTPAD_HERDR_SOCK` env var with the `~/.config/herdr/herdr.sock` default, `-h`/`--help` and no-extra-args handling). Usage text byte-identical to `daemon.py`'s module docstring.

- [ ] **Step 8: Run `cargo build --bin agentpad` then `./target/debug/agentpad --help`**

Expected: prints the usage text.

- [ ] **Step 9: Commit**

```bash
git add src/daemon.rs src/bin/agentpad.rs tests/daemon_actions.rs tests/support/mod.rs
git commit -m "Port daemon.py's AgentPad to daemon.rs and the agentpad binary"
```

### Task 8: diagrams binary

**Files:**
- Create: `src/bin/diagrams.rs`

**Interfaces:**
- Consumes: `crate::daemon::{AgentPad, State, Workspace, LAYER_COLORS, LAYER_NAMES, BOTTOM_KEYS, INACTIVE_DIM, OFF, round_half_even, position}` (all `pub` from Tasks 6-7).
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Port `scripts/diagrams.py` to `src/bin/diagrams.rs`**

Port the SVG-building helpers (`glow`/`text`/`key`/`hexcolor`/`knob`/`pill`/`pad`/`svg`) and the six drawing functions (`layout`/`layers`/`status`/`modes`/`brightness`/`architecture`) plus `main()`. Use `daemon::round_half_even` everywhere Python calls `round()`. Since `State`'s fields are all `pub` (Task 6), build `daemon::State { .. }` struct literals directly in place of Python's local `Example` class — no separate type is needed. Construct `AgentPad::new(None, PathBuf::new(), PathBuf::new())` (the sock path and brightness file are never used, since this binary only calls `.colors()`, never `.press()`/`.run()`) and set `.layer`/`.brightness`/`.all_workspaces` directly (they're `pub`). Pass `now: 0.0` to `colors()` everywhere (matches Python's `daemon.time.monotonic = lambda: 0.0` override in `diagrams.py`'s `main()`, which rendered "blocked" keys in their lit flash phase deterministically). Write output files to `docs/` with `std::fs::write`.

- [ ] **Step 2: Run `cargo build --bin diagrams`**

Expected: success.

- [ ] **Step 3: Run `cargo run --release --bin diagrams`**

- [ ] **Step 4: Run `git diff --stat docs/`**

Expected: no output — the freshly generated SVGs are byte-identical to the ones already committed by the Python tool.

- [ ] **Step 5: If any file differs, fix the port**

Common culprits: float formatting (Python's `f"{x:.2f}"`/`:.0f"` vs Rust's `{:.2}`/`{:.0}`) and any spot still using `.round()` instead of `round_half_even`.

- [ ] **Step 6: Commit**

```bash
git add src/bin/diagrams.rs
git commit -m "Port scripts/diagrams.py to the diagrams binary"
```

### Task 9: .deb packaging for compiled binaries

**Files:**
- Modify: `packaging/deb/control`
- Modify: `scripts/build-deb.sh`
- Modify: `scripts/deb-smoke-test.sh`

**Interfaces:**
- Consumes: nothing new (builds `target/release/{agentpad,side-keyboard-keys,side-keyboard-led}` from Tasks 3, 4, 7).

- [ ] **Step 1: Update `packaging/deb/control`**

`Architecture: all` → `Architecture: amd64`. `Depends: python3 (>= 3.10), udev, systemd` → `Depends: udev, systemd`.

- [ ] **Step 2: Update `scripts/build-deb.sh`**

Replace the Python-install block (installing `src/agentpad/*.py` into `usr/lib/python3/dist-packages/agentpad` and writing shim launchers) with: `cargo build --release --locked`, then `install -D -m 755 target/release/<bin> "$root/usr/bin/<bin>"` for each of `agentpad`, `side-keyboard-keys`, `side-keyboard-led` (not `diagrams` — dev-only, unpackaged). Leave the udev rule install, service file install, README install, control/postinst/prerm copy, and `dpkg-deb --build` steps unchanged.

- [ ] **Step 3: Update `scripts/deb-smoke-test.sh`**

Delete the `python3 -c "import agentpad.daemon"` line. Keep the three `--help`/usage-text greps and the systemd/udev verification lines unchanged (they depend on the Global Constraint that `--help` text is byte-identical to the old Python docstrings).

- [ ] **Step 4: Run `just deb` then `dpkg-deb -c dist/agentpad_*.deb`**

Expected: the file list contains `usr/bin/agentpad`, `usr/bin/side-keyboard-keys`, `usr/bin/side-keyboard-led`, and no `.py` files and no `diagrams` binary.

- [ ] **Step 5: Commit**

```bash
git add packaging/deb/control scripts/build-deb.sh scripts/deb-smoke-test.sh
git commit -m "Package compiled binaries instead of Python sources in the .deb"
```

### Task 10: justfile recipes for Rust

**Files:**
- Modify: `justfile`

**Interfaces:**
- Consumes: the binaries and scripts from Tasks 3-9.

- [ ] **Step 1: `sync`**

`uv sync` → `cargo fetch`.

- [ ] **Step 2: `lint`**

`uv run ruff check .` / `uv run ruff format --check .` → `cargo clippy --all-targets --locked -- -D warnings` then `cargo fmt --check`. Keep the trailing `shellcheck scripts/*.sh packaging/deb/postinst packaging/deb/prerm` line unchanged.

- [ ] **Step 3: `fmt`**

`uv run ruff format .` / `uv run ruff check --fix .` → `cargo fmt` then `cargo clippy --fix --allow-dirty --allow-staged`.

- [ ] **Step 4: `test *args`**

`uv run pytest {{ args }}` → `cargo test --locked {{ args }}`.

- [ ] **Step 5: `diagrams`**

`uv run python scripts/diagrams.py` → `cargo run --release --locked --bin diagrams`.

- [ ] **Step 6: `install`**

Add `bin_dir := env("HOME") / ".local/bin"`. Replace `uv tool install --force --editable .` with `cargo build --release --locked` followed by `install -Dm755 target/release/<bin> {{ bin_dir }}/<bin>` for each of the 3 packaged binaries. Change the `sed` line that rewrites `ExecStart` to point at `{{ bin_dir }}/agentpad` instead of `$(uv tool dir --bin --color never)/agentpad`. Update the comment above the `restart` recipe: a code change now needs `just install` again (rebuild + reinstall) — Rust has no editable-install equivalent, so `just restart` alone no longer picks up edits.

- [ ] **Step 7: Leave `deb`, `deb-test`, `install-deb`, `uninstall`, `logs`, `status` unchanged**

They already call `scripts/build-deb.sh`/`scripts/deb-smoke-test.sh` (Task 9) or systemd directly and don't reference Python.

- [ ] **Step 8: Run `just lint`, `just test`, `just fmt`, `just diagrams`, `just deb`**

Expected: each succeeds (or, for `install`, confirm the recipe's shell syntax with `just --dry-run install` if no pad hardware is available to fully exercise it).

- [ ] **Step 9: Commit**

```bash
git add justfile
git commit -m "Retarget justfile recipes at cargo and the compiled binaries"
```

### Task 11: CI workflow for Rust

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `just lint`/`just test`/`just deb-test` (Task 10).

- [ ] **Step 1: Replace the toolchain setup in `lint` and `test`**

Replace `uses: astral-sh/setup-uv@v10.2.0` with `uses: dtolnay/rust-toolchain@stable` (`with: components: clippy, rustfmt`), and add `uses: Swatinem/rust-cache@v2` for `~/.cargo`/`target` caching.

- [ ] **Step 2: Drop the Python version matrix**

Remove the `test` job's `strategy: matrix: python: [...]` block and its `with: python-version: ${{ matrix.python }}` — a single job on Rust `stable` (no stated MSRV to test against).

- [ ] **Step 3: Keep `run: just lint` / `run: just test -v`**

Unchanged — Task 10 already retargeted what they run.

- [ ] **Step 4: Update the `release` job's version check**

Replace `version=$(python3 -c 'import tomllib; print(tomllib.load(open("pyproject.toml", "rb"))["project"]["version"])')` with `version=$(grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2)`.

- [ ] **Step 5: Remove `env: UV_LOCKED: "1"`**

Task 10's `just` recipes pass `--locked` to `cargo` directly instead.

- [ ] **Step 6: Verify locally**

Run `just lint`, `just test -v`, and `just deb-test` (Task 10's Step 8 already covers `lint`/`test`; run `deb-test` here since it needs Docker) — expect all to succeed, mirroring what CI will run.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "Switch CI from uv/Python to the Rust toolchain"
```

### Task 12: devcontainer

**Files:**
- Create: `.devcontainer/devcontainer.json`

**Interfaces:**
- Consumes: nothing (a fresh dev environment definition).

- [ ] **Step 1: Create `.devcontainer/devcontainer.json`**

```json
{
  "name": "agentpad",
  "image": "ubuntu:24.04",
  "features": {
    "ghcr.io/devcontainers/features/rust:1": {},
    "ghcr.io/SrzStephen/devcontainer-features/just:1": {},
    "ghcr.io/devcontainers/features/docker-outside-of-docker:1": {}
  },
  "postCreateCommand": "sudo apt-get update && sudo apt-get install -y shellcheck && cargo install prek --locked && prek install",
  "customizations": {
    "vscode": {
      "extensions": ["rust-lang.rust-analyzer"]
    }
  }
}
```

`ubuntu:24.04` matches the CI runner and the `.deb`'s target OS exactly. The `just` feature installs both `just` and `just-lsp`. There's no hidraw/input device access inside the container (no physical pad attached there) — same ceiling as CI: build, lint, and test, not run against real hardware.

- [ ] **Step 2: Validate the JSON**

Run: `jq . .devcontainer/devcontainer.json` — expect it to print the parsed document back (no syntax error).

- [ ] **Step 3: If a devcontainer CLI is available, build and verify it**

Run `devcontainer build .` / `devcontainer up --workspace-folder .`, then inside it run `just lint && just test` — expect success. If no devcontainer CLI is available in this environment, note that as a manual follow-up for the human to verify once run locally (e.g. via VS Code's "Reopen in Container" or the Claude Code devcontainer flow).

- [ ] **Step 4: Commit**

```bash
git add .devcontainer/devcontainer.json
git commit -m "Add a devcontainer for Rust development"
```

### Task 13: Cutover — delete Python, update docs

**Files:**
- Delete: `src/agentpad/` (all `.py` files), `tests/test_actions.py`, `tests/test_keys.py`, `tests/test_leds.py`, `tests/test_pad.py`, `tests/conftest.py`, `pyproject.toml`, `uv.lock`
- Modify: `.gitignore`
- Modify: `README.md`
- Modify: `justfile` (`clean` recipe)

**Interfaces:**
- Consumes: nothing (final cleanup task; everything it deletes has already been superseded by Tasks 1-12).

- [ ] **Step 1: Delete the Python source and project files**

```bash
git rm -r src/agentpad pyproject.toml uv.lock
git rm tests/test_actions.py tests/test_keys.py tests/test_leds.py tests/test_pad.py tests/conftest.py
```

Keep `tests/herdr.rs`, `tests/daemon_actions.rs`, `tests/support/`.

- [ ] **Step 2: Remove local Python artifacts from the working tree**

```bash
rm -rf .venv .pytest_cache .ruff_cache
```

- [ ] **Step 3: Update `.gitignore`**

Remove `__pycache__/`, `.venv/`, `.pytest_cache/`, `.ruff_cache/`. Add `/target`.

- [ ] **Step 4: Update the `justfile`'s `clean` recipe**

`rm -rf build dist .pytest_cache .ruff_cache` → `rm -rf dist target`.

- [ ] **Step 5: Update `README.md`**

- "## Install": drop "Python 3.10+" from the requirements line (no runtime Python dependency left). In "From this checkout" (now with `just`, not "with uv"), describe `just install` as: builds a release binary and installs it to `~/.local/bin`; a code change needs `just install` again, not just `just restart` (no editable-install equivalent).
- "## Develop": `just sync # uv sync: .venv with pytest and ruff` → `just sync # cargo fetch`. Replace the "Code lives in `src/agentpad/`..." paragraph with the new `src/` (library modules) / `src/bin/` (binaries) layout. "The version lives in `pyproject.toml`" → "The version lives in `Cargo.toml`."
- "## CI and releases": "lint (ruff, shellcheck), tests on Python 3.10, 3.12 and 3.13" → "lint (`cargo clippy`, `cargo fmt --check`, shellcheck), tests on Rust stable". "matches `pyproject.toml`" → "matches `Cargo.toml`".
- "## Undo": "or with `uv run` from this checkout" → "or from a built checkout: `./target/release/side-keyboard-keys ...`".

- [ ] **Step 6: Run `just check`**

Expected: clean pass (`cargo clippy`, `cargo fmt --check`, `shellcheck`, then `cargo test`).

- [ ] **Step 7: Run `git status` and confirm no Python remains**

Expected: no `__pycache__`, `.venv`, or tracked `*.py` files.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "Remove the Python implementation now that the Rust port is complete"
```
