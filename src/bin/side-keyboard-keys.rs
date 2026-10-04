//! CLI binary for `herdr_vibepad::keys` — see `USAGE` below (byte-identical to
//! `src/agentpad/keys.py`'s module docstring).

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::process::ExitCode;

use herdr_vibepad::keys;
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;

const USAGE: &str =
    "Set what each key and knob on the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc) sends.

Commands are the ones the vendor's WebHID configurator (sdcx-tech.com) sends.
Mappings are stored on the pad, so they work on any computer afterwards.
The pad holds 6 profiles, each a complete mapping; one is active at a time.

  side-keyboard-keys read [--profile N]            # show a profile's mapping
  side-keyboard-keys set SLOT KEY [--profile N]    # map one key or knob action
  side-keyboard-keys all KEY [--profile N]         # map all 16 keys to KEY
  side-keyboard-keys restore [--profile N]         # put back that profile's backup
  side-keyboard-keys profile [N]                   # show / change the active profile

--profile edits a profile without changing which one is active (default: active one).

SLOT: 0-15 for keys (left to right, top to bottom), or knob1-3 with
      .press / .left / .right, e.g. knob1.right
KEY:  a key name with optional modifiers, e.g. f20, a, enter, ctrl+shift+t,
      super+1, a media key: volup, voldown, mute, play, next, prev,
      or profileswitch (steps the pad to its next profile)
      (a raw HID usage like 0x6f also works)

Before the first change to a profile, the script saves its mapping to
~/.side-keyboard-keys-backup-pN.json; `restore` writes it back.
Needs write access to the hidraw node (sudo, or a udev rule).
Never send arbitrary sub-commands: 0x55 and 0x5A put the pad into its bootloader.
";

fn fail(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::from(1)
}

fn usage() -> ExitCode {
    // Matches Python's `sys.exit(__doc__)`: the docstring (which already ends
    // in a newline) printed to stderr, plus `print`'s own trailing newline.
    eprintln!("{USAGE}");
    ExitCode::from(1)
}

fn run(fd: RawFd, cmd: &str, args: &[String], profile: u8) -> Result<(), String> {
    match cmd {
        "read" => {
            for (slot, entry) in keys::read_table(fd)?.into_iter().enumerate() {
                println!("{:<14} {}", keys::slot_name(slot), keys::describe(entry));
            }
            Ok(())
        }
        "set" | "all" => {
            let table = keys::read_table(fd)?;
            keys::save_backup(&table, profile).map_err(|e| e.to_string())?;
            let (slots, key): (Vec<usize>, &str) = if cmd == "set" {
                if args.len() < 2 {
                    return Err("usage: set SLOT KEY".to_string());
                }
                (vec![keys::parse_slot(&args[0])?], &args[1])
            } else {
                if args.is_empty() {
                    return Err("usage: all KEY".to_string());
                }
                ((0..keys::NUM_KEYS).collect(), &args[0])
            };
            let entry = keys::parse_key(key)?;
            for &slot in &slots {
                keys::write_slot(fd, slot, entry)?;
            }
            let after = keys::read_table(fd)?;
            let bad: Vec<String> = slots
                .iter()
                .filter(|&&s| after[s] != entry)
                .map(|&s| keys::slot_name(s))
                .collect();
            if !bad.is_empty() {
                return Err(format!(
                    "pad did not take the new mapping for: {}",
                    bad.join(", ")
                ));
            }
            println!(
                "profile {profile}: set {} slot(s) to {}",
                slots.len(),
                keys::describe(entry)
            );
            Ok(())
        }
        "restore" => {
            let path = keys::backup_path(profile);
            let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let saved: serde_json::Value =
                serde_json::from_str(&data).map_err(|e| e.to_string())?;
            let table = saved
                .get("table")
                .and_then(|t| t.as_array())
                .ok_or_else(|| "backup file missing \"table\"".to_string())?;
            for (slot, entry) in table.iter().enumerate() {
                let bytes = entry
                    .as_array()
                    .ok_or_else(|| "backup file has a malformed slot entry".to_string())?;
                if bytes.len() != 4 {
                    return Err("backup file has a malformed slot entry".to_string());
                }
                let mut e = [0u8; 4];
                for (i, b) in bytes.iter().enumerate() {
                    e[i] = b
                        .as_u64()
                        .ok_or_else(|| "backup file has a malformed slot entry".to_string())?
                        as u8;
                }
                keys::write_slot(fd, slot, e)?;
            }
            println!("profile {profile}: restored");
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return usage();
    }

    let mut profile: Option<u8> = None;
    if let Some(i) = args.iter().position(|a| a == "--profile") {
        if i + 1 >= args.len() {
            return fail("--profile needs a value");
        }
        match args[i + 1].parse::<u8>() {
            Ok(n) => profile = Some(n),
            Err(_) => return fail("--profile needs a numeric value"),
        }
        args.drain(i..i + 2);
    }

    if args.is_empty() {
        return usage();
    }

    let cmd = args.remove(0);

    let device = match keys::find_device() {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    let owned_fd = match open(&device, OFlag::O_RDWR, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) => return fail(&format!("failed to open {}: {e}", device.display())),
    };
    let fd: RawFd = owned_fd.as_raw_fd();

    let (mut active, mut count) = match keys::get_profile(fd) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };

    if cmd == "profile" {
        if let Some(n) = args.first() {
            let n: u8 = match n.parse() {
                Ok(n) => n,
                Err(_) => return fail("profile number must be an integer"),
            };
            if let Err(e) = keys::select_profile(fd, n) {
                return fail(&e);
            }
            match keys::get_profile(fd) {
                Ok((a, c)) => {
                    active = a;
                    count = c;
                }
                Err(e) => return fail(&e),
            }
        }
        println!("active profile: {active} (of 0-{})", count - 1);
        return ExitCode::SUCCESS;
    }

    let profile = profile.unwrap_or(active);
    if profile >= count {
        return fail(&format!("profile must be 0-{}", count - 1));
    }
    if profile != active {
        if let Err(e) = keys::select_profile(fd, profile) {
            return fail(&e);
        }
    }

    let result = run(fd, &cmd, &args, profile);

    if profile != active {
        if let Err(e) = keys::select_profile(fd, active) {
            if result.is_ok() {
                return fail(&e);
            }
        }
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e == USAGE => usage(),
        Err(e) => fail(&e),
    }
}
