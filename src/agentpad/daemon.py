"""Drive herdr workspaces and agents from the SDINNOVATION SIDE-KEYBOARD (6d7d:dcfc).

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
"""

import errno
import fcntl
import glob
import json
import os
import select
import socket
import sys
import time

from agentpad import keys
from agentpad import led as leds

HERDR_SOCK = os.environ.get("AGENTPAD_HERDR_SOCK", os.path.expanduser("~/.config/herdr/herdr.sock"))
PAD_PROFILE = 5
LED_HW_BRIGHTNESS = 4  # the pad's own brightness (0-4); colours are scaled in software
POLL = 0.25  # seconds between herdr state polls
FLASH = 0.5  # seconds per on/off half of the blocked flash

AGENT_KEYS = 12
BOTTOM_KEYS = {  # layer -> herdr key names for keys 12-15, None = unmapped
    1: ["1", "2", "3", "esc"],
    2: ["1", "2", "3", "esc"],
    3: ["y", "n", "t", "esc"],
}
LAYER_COLORS = {1: (255, 50, 0), 2: (0, 60, 255), 3: (150, 0, 255)}
LAYER_NAMES = {1: "Claude", 2: "Codex", 3: "Kiro"}  # the agent each layer's keys suit
BOTTOM_BRIGHTNESS = 0.2  # bottom row, as a fraction of the layer colour
STATUS_COLORS = {  # herdr agent_status -> colour; anything else shows as idle
    "working": (255, 160, 0),
    "blocked": (255, 0, 0),  # waiting on an approval or question; flashes
    "done": (0, 255, 0),  # finished and not yet looked at
    "idle": (255, 255, 255),
}
TRIPLE_PRESS = 3  # knob 1 presses that toggle all-workspaces mode...
TRIPLE_PRESS_WINDOW = 1.0  # ...within this many seconds
BRIGHTNESS_STEP = 5  # knob 3: percentage points per click
BRIGHTNESS_MIN = 5  # so the pad never looks switched off
BRIGHTNESS_FILE = os.path.expanduser("~/.local/state/agentpad-brightness")
INACTIVE_DIM = 5  # agents other than the active one are this many times dimmer
OFF = (0, 0, 0)

# What each pad slot sends: keys 0-15, then knob1-3 x press/right/left.
SLOT_KEYS = [f"f{n}" for n in range(13, 25)] + [f"shift+f{n}" for n in range(13, 25)] + ["ctrl+f13"]

EVIOCGRAB = 0x40044590


def position(i):
    """Physical position (left to right, top to bottom) of pad key/LED index i.

    The pad numbers keys and LEDs bottom to top, left column first."""
    return (3 - i % 4) * 4 + i // 4


SLOT_ENTRIES = [keys.parse_key(k) for k in SLOT_KEYS]


# The kernel drops F13-F24 from this pad's keyboard interface, so key reports are
# read raw from hidraw: [modifiers, 0, usage x 6]. Map (modifiers, usage) -> slot.
CODE_TO_SLOT = {(e[1], e[2]): slot for slot, e in enumerate(SLOT_ENTRIES)}


class PadGone(Exception):
    pass


_last_log = None


def log(msg, once=False):
    """Print a message; with once, skip it if it repeats the previous one."""
    global _last_log
    if once and msg == _last_log:
        return
    _last_log = msg
    print(msg, flush=True)


# ---------------------------------------------------------------- herdr


def herdr(method, **params):
    try:
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(2)
            s.connect(HERDR_SOCK)
            s.sendall((json.dumps({"id": "agentpad", "method": method, "params": params}) + "\n").encode())
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = s.recv(65536)
                if not chunk:
                    break
                buf += chunk
        reply = json.loads(buf)
    except (OSError, ValueError) as e:
        log(f"herdr {method}: {e}", once=True)
        return None
    if "error" in reply:
        log(f"herdr {method}: {reply['error']}")
        return None
    return reply["result"]


def id_number(ident):
    return int(ident.rsplit(":", 1)[1][1:])  # "w1:t2" -> 2, "w1:p3" -> 3


class State:
    """What the pad needs to know about herdr right now."""

    def __init__(self):
        ws = herdr("workspace.list")
        ag = herdr("agent.list")
        self.workspaces = sorted(ws["workspaces"], key=lambda w: w["number"]) if ws else []
        self.workspace = next((w["workspace_id"] for w in self.workspaces if w["focused"]), None)
        number = {w["workspace_id"]: w["number"] for w in self.workspaces}
        agents = sorted(
            ag["agents"] if ag else [],
            key=lambda a: (
                number.get(a["workspace_id"], 0),
                id_number(a["tab_id"]),
                id_number(a["pane_id"]),
            ),
        )
        self.all_agents = [a["pane_id"] for a in agents]  # every workspace, in workspace order
        self.agents = [a["pane_id"] for a in agents if a["workspace_id"] == self.workspace]
        self.status = {a["pane_id"]: a["agent_status"] for a in agents}
        self.active = next((a["pane_id"] for a in agents if a["focused"]), None)


def load_brightness():
    try:
        with open(BRIGHTNESS_FILE) as f:
            return min(100, max(BRIGHTNESS_MIN, int(f.read())))
    except (OSError, ValueError):
        return 100


def save_brightness(pct):
    try:
        os.makedirs(os.path.dirname(BRIGHTNESS_FILE), exist_ok=True)
        with open(BRIGHTNESS_FILE, "w") as f:
            f.write(f"{pct}\n")
    except OSError as e:
        log(f"can't save brightness: {e}")


def step(items, current, delta):
    if not items:
        return None
    if current not in items:
        return items[0] if delta > 0 else items[-1]
    return items[(items.index(current) + delta) % len(items)]


# ---------------------------------------------------------------- pad


class Pad:
    def __init__(self):
        self.hid = self.reports = None
        self.inputs = []  # grabbed only so the pad's codes don't reach the desktop
        self.down = []  # usages held in the last key report
        self.frame = None
        try:
            self.hid = os.open(keys.find_device(), os.O_RDWR)
            self.reports = os.open(report_node(), os.O_RDONLY | os.O_NONBLOCK)
            for path in input_nodes():
                fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
                self.inputs.append(fd)
                fcntl.ioctl(fd, EVIOCGRAB, 1)
            if not self.inputs:  # e.g. WSL without evdev; there's no desktop to shield
                log("pad input devices not found; its keys are not grabbed", once=True)
            self.prepare()
        except PadGone:
            self.close()
            raise
        except SystemExit as e:  # the protocol modules exit on errors
            self.close()
            raise PadGone(str(e.code)) from None
        except OSError as e:
            self.close()
            raise PadGone(str(e)) from e

    def close(self):
        for fd in [self.hid, self.reports, *self.inputs]:
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
        self.hid = self.reports = None
        self.inputs = []

    def drain(self):
        while select.select([self.hid], [], [], 0)[0]:
            os.read(self.hid, 64)

    def prepare(self):
        """Select our profile, make its slots send our codes, set LEDs to per-key at full brightness."""
        self.drain()
        active, _ = keys.get_profile(self.hid)
        if active != PAD_PROFILE:
            try:
                keys.select_profile(self.hid, PAD_PROFILE)
            except SystemExit:
                pass  # the pad sometimes switches without replying
            self.drain()
            if keys.get_profile(self.hid)[0] != PAD_PROFILE:
                raise PadGone(f"pad did not switch to profile {PAD_PROFILE}")
            log(f"pad switched from profile {active} to {PAD_PROFILE}")
        table = keys.read_table(self.hid)
        wrong = [slot for slot, want in enumerate(SLOT_ENTRIES) if table[slot] != want]
        if wrong:
            keys.save_backup(table, PAD_PROFILE)
            for slot in wrong:
                keys.write_slot(self.hid, slot, SLOT_ENTRIES[slot])
            log(f"programmed {len(wrong)} slot(s) of profile {PAD_PROFILE}")
        self.drain()
        state = leds.read_state(self.hid)
        if state[2] != leds.CUSTOM or state[3] != LED_HW_BRIGHTNESS:
            state[2], state[3] = leds.CUSTOM, LED_HW_BRIGHTNESS
            leds.write_state(self.hid, state)

    def show(self, colors):
        if colors == self.frame:
            return
        data = [c for i in range(keys.NUM_KEYS) for c in colors[position(i)]]
        # Same bulk per-key write as led.set_all_keys; 48 bytes fit one frame.
        leds.send(self.hid, [0x06, 0x12, len(data) + 3, 0, 0, 0, 0, 0] + data)
        self.frame = colors

    def events(self, timeout):
        """Wait up to timeout and return the slots pressed meanwhile."""
        fds = [self.hid, self.reports, *self.inputs]
        slots = []
        try:
            for fd in select.select(fds, [], [], timeout)[0]:
                try:
                    data = os.read(fd, 4096)  # command replies and grabbed events are discarded
                except BlockingIOError:
                    continue
                if fd != self.reports or len(data) < 3:
                    continue
                usages = [u for u in data[2:8] if u]
                for u in usages:
                    if u not in self.down:
                        slot = CODE_TO_SLOT.get((data[0], u))
                        if slot is not None:
                            slots.append(slot)
                self.down = usages
        except OSError as e:
            if e.errno in (errno.ENODEV, errno.EIO, errno.EBADF):
                raise PadGone("pad disconnected") from e
            raise
        return slots


def report_node():
    """The hidraw node of the interface that carries the pad's key reports."""
    for d in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        with open(d + "/device/uevent") as f:
            u = f.read()
        if "00006D7D:0000DCFC" in u and "/input1\n" in u:
            return "/dev/" + os.path.basename(d)
    raise PadGone("pad key interface not found")


def input_nodes():
    nodes = []
    for d in glob.glob("/sys/class/input/event*"):
        try:
            with open(d + "/device/id/vendor") as v, open(d + "/device/id/product") as p:
                if (v.read().strip(), p.read().strip()) == ("6d7d", "dcfc"):
                    nodes.append("/dev/input/" + os.path.basename(d))
        except OSError:
            pass
    return sorted(nodes)


# ---------------------------------------------------------------- behaviour


class AgentPad:
    def __init__(self, pad):
        self.pad = pad
        self.layer = 1
        self.brightness = load_brightness()
        self.all_workspaces = False  # agent keys cover every workspace, not just the focused one
        self.knob1_presses = []

    def keyed_agents(self, st):
        """The agents the top three rows stand for, in key order."""
        return st.all_agents if self.all_workspaces else st.agents

    def press(self, slot, st):
        if slot < keys.NUM_KEYS:
            pos = position(slot)
            log(f"key row {pos // 4 + 1} column {pos % 4 + 1}")
            self.key(pos, st)
        else:
            knob, part = divmod(slot - keys.NUM_KEYS, 3)
            self.knob(knob + 1, keys.KNOB_PARTS[part], st)

    def key(self, pos, st):
        """pos: physical key position, left to right, top to bottom."""
        if pos < AGENT_KEYS:
            agents = self.keyed_agents(st)
            if pos < len(agents):
                herdr("agent.focus", target=agents[pos])  # also focuses its workspace
        else:
            key = BOTTOM_KEYS[self.layer][pos - AGENT_KEYS]
            if key and st.active:
                # agent.send_keys only takes named agents; the pane works for any.
                herdr("pane.send_keys", pane_id=st.active, keys=[key])

    def knob(self, n, action, st):
        if action == "press":
            self.layer = n
            log(f"layer {self.layer} ({LAYER_NAMES[self.layer]} mode)")
            if n == 1:
                self.count_knob1_press()
        elif n == 1:
            ids = [w["workspace_id"] for w in st.workspaces]
            target = step(ids, st.workspace, 1 if action == "right" else -1)
            if target and target != st.workspace:
                herdr("workspace.focus", workspace_id=target)
        elif n == 2:
            target = step(st.agents, st.active, 1 if action == "right" else -1)
            if target and target != st.active:
                herdr("agent.focus", target=target)
        elif n == 3:
            step_pct = BRIGHTNESS_STEP if action == "right" else -BRIGHTNESS_STEP
            brightness = min(100, max(BRIGHTNESS_MIN, self.brightness + step_pct))
            if brightness != self.brightness:
                self.brightness = brightness
                save_brightness(brightness)
                log(f"brightness {brightness}%")

    def count_knob1_press(self):
        """Pressing knob 1 TRIPLE_PRESS times within TRIPLE_PRESS_WINDOW toggles all-workspaces mode."""
        now = time.monotonic()
        self.knob1_presses = [t for t in self.knob1_presses if now - t < TRIPLE_PRESS_WINDOW] + [now]
        if len(self.knob1_presses) >= TRIPLE_PRESS:
            self.knob1_presses = []
            self.all_workspaces = not self.all_workspaces
            log("agent keys: " + ("all workspaces" if self.all_workspaces else "focused workspace"))

    def colors(self, st):
        layer = LAYER_COLORS[self.layer]
        flash_on = int(time.monotonic() / FLASH) % 2 == 0
        agents = self.keyed_agents(st)
        out = []
        for k in range(keys.NUM_KEYS):
            if k < AGENT_KEYS:
                agent = agents[k] if k < len(agents) else None
                status = st.status.get(agent)
                if not agent or (status == "blocked" and not flash_on):
                    out.append(OFF)
                    continue
                rgb = STATUS_COLORS.get(status, STATUS_COLORS["idle"])
                out.append(rgb if agent == st.active else tuple(c // INACTIVE_DIM for c in rgb))
            else:
                out.append(tuple(round(c * BOTTOM_BRIGHTNESS) for c in layer))
        return [tuple(round(c * self.brightness / 100) for c in rgb) for rgb in out]

    def run(self):
        st = State()
        self.pad.show(self.colors(st))
        next_poll = time.monotonic() + POLL
        while True:
            slots = self.pad.events(max(0, next_poll - time.monotonic()))
            for slot in slots:
                self.press(slot, st)
                st = State()
            if not slots:
                st = State()
                next_poll = time.monotonic() + POLL
            self.pad.show(self.colors(st))


def main():
    if sys.argv[1:] in (["-h"], ["--help"]):
        print(__doc__)
        return
    if len(sys.argv) > 1:
        sys.exit(__doc__)
    while True:
        try:
            pad = Pad()
        except PadGone as e:
            log(f"waiting for pad: {e}", once=True)
            time.sleep(2)
            continue
        log(f"pad ready, profile {PAD_PROFILE}")
        try:
            AgentPad(pad).run()
        except PadGone as e:
            log(str(e))
        finally:
            pad.close()
        time.sleep(1)


if __name__ == "__main__":
    main()
