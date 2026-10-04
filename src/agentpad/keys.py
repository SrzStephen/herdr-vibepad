"""Set what each key and knob on the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc) sends.

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
"""

import glob
import json
import os
import select
import sys

NUM_SLOTS = 25  # 16 keys + 3 knobs x 3 actions
NUM_KEYS = 16
LAYER = 0  # the pad reports one layer per profile
HOME = os.path.expanduser("~" + os.environ.get("SUDO_USER", ""))

TYPE_KEY, TYPE_CONSUMER, TYPE_DISABLED, TYPE_MACRO = 0x20, 0x30, 0x13, 0x60
TYPE_FUNCTION = 0x1F  # actions the pad performs itself

MODIFIERS = {
    "ctrl": 0x01,
    "shift": 0x02,
    "alt": 0x04,
    "super": 0x08,
    "win": 0x08,
    "gui": 0x08,
    "rctrl": 0x10,
    "rshift": 0x20,
    "ralt": 0x40,
    "rsuper": 0x80,
}

USAGES = {chr(ord("a") + i): 0x04 + i for i in range(26)}
USAGES.update({str(i): 0x1E + i - 1 for i in range(1, 10)})
USAGES["0"] = 0x27
USAGES.update({f"f{i}": 0x3A + i - 1 for i in range(1, 13)})
USAGES.update({f"f{i}": 0x68 + i - 13 for i in range(13, 25)})
USAGES.update(
    {
        "enter": 0x28,
        "esc": 0x29,
        "backspace": 0x2A,
        "tab": 0x2B,
        "space": 0x2C,
        "minus": 0x2D,
        "equal": 0x2E,
        "lbracket": 0x2F,
        "rbracket": 0x30,
        "backslash": 0x31,
        "semicolon": 0x33,
        "quote": 0x34,
        "grave": 0x35,
        "comma": 0x36,
        "dot": 0x37,
        "slash": 0x38,
        "capslock": 0x39,
        "printscreen": 0x46,
        "scrolllock": 0x47,
        "pause": 0x48,
        "insert": 0x49,
        "home": 0x4A,
        "pageup": 0x4B,
        "delete": 0x4C,
        "end": 0x4D,
        "pagedown": 0x4E,
        "right": 0x4F,
        "left": 0x50,
        "down": 0x51,
        "up": 0x52,
        "menu": 0x65,
    }
)
CONSUMER = {
    "volup": 0xE9,
    "voldown": 0xEA,
    "mute": 0xE2,
    "play": 0xCD,
    "next": 0xB5,
    "prev": 0xB6,
    "stop": 0xB7,
}
FUNCTIONS = {"profileswitch": 0x13}

KNOB_PARTS = ["press", "right", "left"]  # order of a knob's three slots


def find_device():
    for d in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        with open(d + "/device/uevent") as f:
            u = f.read()
        # Interface 2 is the vendor config channel.
        if "00006D7D:0000DCFC" in u and "/input2\n" in u:
            return "/dev/" + os.path.basename(d)
    sys.exit("SIDE-KEYBOARD config interface not found")


def send(fd, payload):
    frame = bytes(payload) + bytes(64 - len(payload))
    os.write(fd, b"\x00" + frame)  # leading 0 = unnumbered report


def request(fd, payload, reply_cmds, what):
    """Send a command and wait for its reply; the pad needs one at a time."""
    send(fd, payload)
    while True:
        if not select.select([fd], [], [], 1.0)[0]:
            sys.exit(f"no reply from device ({what})")
        r = os.read(fd, 64)
        if len(r) >= 8 and r[0] == 0xAA and r[1] in reply_cmds:
            return r


def get_profile(fd):
    r = request(fd, [0x06, 0x05], (0x05,), "device info")
    return r[16], r[15]  # current, count


def select_profile(fd, n):
    request(fd, [0x06, 0xFB, n], (0xFB,), f"select profile {n}")


def read_table(fd):
    table = []
    for off in range(0, 4 * NUM_SLOTS, 56):
        while True:
            # The pad answers a 0x08 read with an 0x07 header.
            r = request(fd, [0x06, 0x08, 0x3A, off & 0xFF, off >> 8, 0, LAYER], (0x07, 0x08), "read keys")
            if r[3:5] == bytes([off & 0xFF, off >> 8]):
                break
        table += list(r[8:64])
    table = table[: 4 * NUM_SLOTS]
    return [table[i : i + 4] for i in range(0, len(table), 4)]


def write_slot(fd, slot, entry):
    off = 4 * slot
    # Back-to-back writes without waiting for the ack corrupt the table.
    request(
        fd,
        [0x06, 0x10, 0x07, off & 0xFF, off >> 8, 0, LAYER, 0] + list(entry),
        (0x10,),
        f"write {slot_name(slot)}",
    )


def parse_slot(text):
    if text.isdigit() and int(text) < NUM_KEYS:
        return int(text)
    if text.startswith("knob") and "." in text:
        knob, part = text[4:].split(".", 1)
        if knob in ("1", "2", "3") and part in KNOB_PARTS:
            return NUM_KEYS + 3 * (int(knob) - 1) + KNOB_PARTS.index(part)
    sys.exit(f"bad slot {text!r}: use 0-15 or knob1-3.press/left/right")


def slot_name(slot):
    if slot < NUM_KEYS:
        return f"key {slot}"
    k, p = divmod(slot - NUM_KEYS, 3)
    return f"knob{k + 1}.{KNOB_PARTS[p]}"


def parse_key(text):
    parts = text.lower().split("+")
    mods = 0
    for m in parts[:-1]:
        if m not in MODIFIERS:
            sys.exit(f"unknown modifier {m!r}")
        mods |= MODIFIERS[m]
    key = parts[-1]
    if key in FUNCTIONS and not mods:
        return [TYPE_FUNCTION, FUNCTIONS[key], 0, 0]
    if key in CONSUMER and not mods:
        code = CONSUMER[key]
        return [TYPE_CONSUMER, code & 0xFF, code >> 8, 0]
    if key in USAGES:
        return [TYPE_KEY, mods, USAGES[key], 0]
    if key.startswith("0x"):
        return [TYPE_KEY, mods, int(key, 16), 0]
    sys.exit(f"unknown key {key!r}")


def describe(entry):
    t, c1, c2, c3 = entry
    if t == TYPE_KEY:
        name = next((k for k, v in USAGES.items() if v == c2), f"0x{c2:02x}")
        mods = [k for k, v in MODIFIERS.items() if c1 & v and k not in ("win", "gui")]
        return "+".join(mods + [name])
    if t == TYPE_CONSUMER:
        code = c1 | c2 << 8
        return next((k for k, v in CONSUMER.items() if v == code), f"media 0x{code:04x}")
    if t == TYPE_FUNCTION:
        return next((k for k, v in FUNCTIONS.items() if v == c1), f"function 0x{c1:02x}")
    if t == TYPE_MACRO:
        return f"macro M{c1}"
    if t == TYPE_DISABLED:
        return "(disabled)"
    return f"type 0x{t:02x}: {c1:02x} {c2:02x} {c3:02x}"


def backup_path(profile):
    return f"{HOME}/.side-keyboard-keys-backup-p{profile}.json"


def save_backup(table, profile):
    path = backup_path(profile)
    if os.path.exists(path):
        return
    with open(path, "w") as f:
        json.dump({"profile": profile, "layer": LAYER, "table": table}, f)
    if os.environ.get("SUDO_UID"):
        os.chown(path, int(os.environ["SUDO_UID"]), int(os.environ["SUDO_GID"]))
    print(f"saved profile {profile}'s original mapping to {path}")


def run(fd, cmd, args, profile):
    if cmd == "read":
        for slot, entry in enumerate(read_table(fd)):
            print(f"{slot_name(slot):14} {describe(entry)}")
    elif cmd in ("set", "all"):
        save_backup(read_table(fd), profile)
        if cmd == "set":
            slots, key = [parse_slot(args[0])], args[1]
        else:
            slots, key = range(NUM_KEYS), args[0]
        entry = parse_key(key)
        for slot in slots:
            write_slot(fd, slot, entry)
        after = read_table(fd)
        bad = [slot_name(s) for s in slots if after[s] != entry]
        if bad:
            sys.exit("pad did not take the new mapping for: " + ", ".join(bad))
        print(f"profile {profile}: set {len(slots)} slot(s) to {describe(entry)}")
    elif cmd == "restore":
        with open(backup_path(profile)) as f:
            saved = json.load(f)
        for slot, entry in enumerate(saved["table"]):
            write_slot(fd, slot, entry)
        print(f"profile {profile}: restored")
    else:
        sys.exit(__doc__)


def main():
    args = sys.argv[1:]
    profile = None
    if "--profile" in args:
        i = args.index("--profile")
        profile = int(args[i + 1])
        del args[i : i + 2]
    if not args:
        sys.exit(__doc__)
    cmd, args = args[0], args[1:]
    fd = os.open(find_device(), os.O_RDWR)
    try:
        active, count = get_profile(fd)
        if cmd == "profile":
            if args:
                select_profile(fd, int(args[0]))
                active, count = get_profile(fd)
            print(f"active profile: {active} (of 0-{count - 1})")
            return
        if profile is None:
            profile = active
        if not 0 <= profile < count:
            sys.exit(f"profile must be 0-{count - 1}")
        if profile != active:
            select_profile(fd, profile)
        try:
            run(fd, cmd, args, profile)
        finally:
            if profile != active:
                select_profile(fd, active)
    finally:
        os.close(fd)


if __name__ == "__main__":
    main()
