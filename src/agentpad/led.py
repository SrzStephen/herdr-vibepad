"""Control the LEDs on the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc, 16 keys + 3 knobs).

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
"""

import glob
import os
import select
import sys
import time

MODES = ["off", "solid", "breathing", "light-on-press", "tide", "custom"]
CUSTOM = 5
NUM_KEYS = 16


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


def read_state(fd):
    send(fd, [0x06, 0x0A])
    while select.select([fd], [], [], 1.0)[0]:
        r = os.read(fd, 64)
        if len(r) >= 16 and r[0] == 0xAA:
            return list(r[5:16])
    sys.exit("no backlight reply from device")


def write_state(fd, s):
    # s = [type, 0, mode, brightness, speed, direction, color, 0, h, s, v]
    if s[2] == 0:
        s[6] = 0
    send(fd, [0x06, 0x0B, len(s), 0x00, 0x00] + s)


def set_key(fd, index, rgb):
    off = 3 * index
    send(fd, [0x06, 0x14, 3, off & 0xFF, off >> 8, 0, 0, 0] + rgb)


def set_all_keys(fd, rgb):
    data = rgb * NUM_KEYS
    for start in range(0, len(data), 56):
        chunk = data[start : start + 56]
        send(fd, [0x06, 0x12, len(chunk) + 3, start & 0xFF, start >> 8, 0, 0, 0] + chunk)


def ensure_custom(fd, cur):
    if cur[2] != CUSTOM or cur[3] == 0:
        new = list(cur)
        new[2], new[3] = CUSTOM, 4
        write_state(fd, new)
        set_all_keys(fd, [0, 0, 0])


def parse_rgb(hexstr):
    v = int(hexstr.lstrip("#"), 16)
    return [v >> 16 & 0xFF, v >> 8 & 0xFF, v & 0xFF]


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    cmd, args = sys.argv[1], sys.argv[2:]
    fd = os.open(find_device(), os.O_RDWR)
    try:
        cur = read_state(fd)
        if cmd == "read":
            name = MODES[cur[2]] if cur[2] < len(MODES) else cur[2]
            print(
                f"mode={name} brightness={cur[3]} speed={cur[4]} direction={cur[5]} "
                f"color={cur[6]} hsv=({cur[8]},{cur[9]},{cur[10]})"
            )
            print("restore with: raw " + " ".join(f"{b:02x}" for b in cur))
        elif cmd == "raw":
            write_state(fd, [int(x, 16) for x in args])
        elif cmd == "set":
            mode = int(args[0])
            new = list(cur)
            new[2] = mode
            if new[3] == 0:
                new[3] = 4  # brightness 0 is dark
            if len(args) > 1:
                new[6], new[8], new[9], new[10] = 1, int(args[1]), 255, 255
            if len(args) > 2:
                new[4] = min(int(args[2]), 4)
            write_state(fd, new)
        elif cmd == "key":
            ensure_custom(fd, cur)
            set_key(fd, int(args[0]), parse_rgb(args[1]))
        elif cmd == "flash":
            index, rgb = int(args[0]), parse_rgb(args[1])
            count = int(args[2]) if len(args) > 2 else 10
            period = float(args[3]) if len(args) > 3 else 0.5
            ensure_custom(fd, cur)
            for _ in range(count):
                set_key(fd, index, rgb)
                time.sleep(period / 2)
                set_key(fd, index, [0, 0, 0])
                time.sleep(period / 2)
        else:
            sys.exit(__doc__)
    finally:
        os.close(fd)


if __name__ == "__main__":
    main()
