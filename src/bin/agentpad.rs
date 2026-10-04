//! CLI binary for `agentpad::daemon` — see `USAGE` below (byte-identical to
//! `src/agentpad/daemon.py`'s module docstring).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use agentpad::daemon::{AgentPad, Pad, PAD_PROFILE};

const USAGE: &str =
    "Drive herdr workspaces and agents from the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc).

  knob 1 turn     previous / next herdr workspace
  knob 2 turn     previous / next agent in the focused workspace
  knob 3 turn     all LEDs brighter / dimmer, BRIGHTNESS_STEP % per click
  knob N press    switch to layer N (1-3)
  knob 1 x3       (within a second) toggle agent keys between the focused workspace
                  and every workspace; in the latter a key jumps to its agent's workspace
  top 3 rows      key N (left to right, top to bottom) focuses agent N of the workspace
  bottom row      sends keys to the active agent:
                    layer 1: 1 2 3 esc    layer 2: 1 2 3 -    layer 3: y n t -

LEDs: the bottom row shows the layer colour (reddish orange / blue / purple)
at 20% brightness. An agent's key shows its status:
yellow working, flashing red waiting for you, green done, white idle. The
active agent's key is full brightness, the others dimmed.

The pad is put on profile PAD_PROFILE, whose 25 slots are (re)programmed to send
F13-F24 codes. The kernel drops those from the pad's keyboard interface, so key
reports are read raw from its hidraw node; its input devices are still grabbed so
nothing reaches the desktop. The pad numbers its keys and LEDs column by
column from the bottom left. Everything else happens here, through herdr's
socket API.

  agentpad               run the daemon (it waits for the pad and for herdr)

Needs access to the pad's hidraw and input nodes: see 70-side-keyboard.rules.
";

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

fn herdr_sock_path() -> PathBuf {
    match std::env::var("AGENTPAD_HERDR_SOCK") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(format!("{}/.config/herdr/herdr.sock", home_dir())),
    }
}

fn brightness_file_path() -> PathBuf {
    PathBuf::from(format!("{}/.local/state/agentpad-brightness", home_dir()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args == ["-h"] || args == ["--help"] {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if !args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(1);
    }

    let sock_path = herdr_sock_path();
    let brightness_file = brightness_file_path();

    loop {
        let pad = match Pad::open() {
            Ok(p) => p,
            Err(e) => {
                agentpad::log(&format!("waiting for pad: {e}"), true);
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        agentpad::log(&format!("pad ready, profile {PAD_PROFILE}"), false);
        let mut agent_pad = AgentPad::new(Some(pad), sock_path.clone(), brightness_file.clone());
        if let Err(e) = agent_pad.run() {
            agentpad::log(&format!("{e}"), false);
        }
        // `agent_pad`'s `Option<Pad>` is dropped here, closing all its file
        // descriptors (matches Python's `pad.close()` in `finally`).
        std::thread::sleep(Duration::from_secs(1));
    }
}
