//! CLI binary for `herdr_vibepad::led` — see `USAGE` below (byte-identical to
//! `src/agentpad/led.py`'s module docstring).

use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use herdr_vibepad::led;
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;

const USAGE: &str =
    "Control the LEDs on the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc, 16 keys + 3 knobs).

Commands are the ones the vendor's WebHID configurator (sdcx-tech.com) sends.
The firmware has no blink effect, so `flash` toggles a key's colour from here.

  side-keyboard-led read                       # show current state (save it!)
  side-keyboard-led set MODE [hue] [speed]     # whole-pad effect, see MODES below
  side-keyboard-led key INDEX RRGGBB           # one key's colour (custom mode)
  side-keyboard-led flash INDEX RRGGBB [count] [seconds]
  side-keyboard-led raw HEX...                 # restore bytes printed by `read`

MODES: 0 off, 1 solid, 2 breathing, 3 light-on-press, 4 tide, 5 custom (per-key).
Keys are numbered 0-15 left to right, top to bottom. hue 0-255, speed 0-4.

Needs write access to the hidraw node (sudo, or a udev rule).
Never send arbitrary sub-commands: 0x55 and 0x5A put the pad into its bootloader.
";

const MODES: [&str; 6] = [
    "off",
    "solid",
    "breathing",
    "light-on-press",
    "tide",
    "custom",
];

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

fn run(fd: RawFd, cmd: &str, args: &[String], cur: [u8; 11]) -> Result<(), String> {
    match cmd {
        "read" => {
            let name = if (cur[2] as usize) < MODES.len() {
                MODES[cur[2] as usize].to_string()
            } else {
                cur[2].to_string()
            };
            println!(
                "mode={name} brightness={} speed={} direction={} color={} hsv=({},{},{})",
                cur[3], cur[4], cur[5], cur[6], cur[8], cur[9], cur[10]
            );
            let hex: Vec<String> = cur.iter().map(|b| format!("{b:02x}")).collect();
            println!("restore with: raw {}", hex.join(" "));
            Ok(())
        }
        "raw" => {
            let mut vals: Vec<u8> = Vec::with_capacity(args.len());
            for a in args {
                let v = u8::from_str_radix(a, 16).map_err(|_| format!("bad hex byte {a:?}"))?;
                vals.push(v);
            }
            if vals.len() > 6 && vals[2] == 0 {
                vals[6] = 0;
            }
            let mut payload = vec![0x06, 0x0B, vals.len() as u8, 0x00, 0x00];
            payload.extend_from_slice(&vals);
            herdr_vibepad::hid::send(fd, &payload).map_err(|e| e.to_string())
        }
        "set" => {
            if args.is_empty() {
                return Err("usage: set MODE [hue] [speed]".to_string());
            }
            let mode: u8 = args[0]
                .parse()
                .map_err(|_| format!("bad mode {:?}", args[0]))?;
            let mut new = cur;
            new[2] = mode;
            if new[3] == 0 {
                new[3] = 4; // brightness 0 is dark
            }
            if args.len() > 1 {
                let hue: u8 = args[1]
                    .parse()
                    .map_err(|_| format!("bad hue {:?}", args[1]))?;
                new[6] = 1;
                new[8] = hue;
                new[9] = 255;
                new[10] = 255;
            }
            if args.len() > 2 {
                let speed: u8 = args[2]
                    .parse()
                    .map_err(|_| format!("bad speed {:?}", args[2]))?;
                new[4] = speed.min(4);
            }
            led::write_state(fd, &mut new);
            Ok(())
        }
        "key" => {
            if args.len() < 2 {
                return Err("usage: key INDEX RRGGBB".to_string());
            }
            let index: usize = args[0]
                .parse()
                .map_err(|_| format!("bad index {:?}", args[0]))?;
            let rgb = led::parse_rgb(&args[1])?;
            led::ensure_custom(fd, &cur).map_err(|e| e.to_string())?;
            led::set_key(fd, index, rgb).map_err(|e| e.to_string())
        }
        "flash" => {
            if args.len() < 2 {
                return Err("usage: flash INDEX RRGGBB [count] [seconds]".to_string());
            }
            let index: usize = args[0]
                .parse()
                .map_err(|_| format!("bad index {:?}", args[0]))?;
            let rgb = led::parse_rgb(&args[1])?;
            let count: u32 = if args.len() > 2 {
                args[2]
                    .parse()
                    .map_err(|_| format!("bad count {:?}", args[2]))?
            } else {
                10
            };
            let period: f64 = if args.len() > 3 {
                args[3]
                    .parse()
                    .map_err(|_| format!("bad seconds {:?}", args[3]))?
            } else {
                0.5
            };
            led::ensure_custom(fd, &cur).map_err(|e| e.to_string())?;
            for _ in 0..count {
                led::set_key(fd, index, rgb).map_err(|e| e.to_string())?;
                thread::sleep(Duration::from_secs_f64(period / 2.0));
                led::set_key(fd, index, [0, 0, 0]).map_err(|e| e.to_string())?;
                thread::sleep(Duration::from_secs_f64(period / 2.0));
            }
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        return usage();
    }

    let cmd = &args[0];
    let rest = &args[1..];

    let device = match led::find_device() {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    let owned_fd = match open(&device, OFlag::O_RDWR, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) => return fail(&format!("failed to open {}: {e}", device.display())),
    };
    let fd: RawFd = owned_fd.as_raw_fd();

    let cur = match led::read_state(fd) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };

    match run(fd, cmd, rest, cur) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}
