"""Generate the README's SVG diagrams in docs/ from the daemon's own layout and colours.

Run with `just diagrams` after changing colours, layers or the key layout.
"""

from pathlib import Path
from xml.sax.saxutils import escape

from agentpad import daemon

DOCS = Path(__file__).resolve().parent.parent / "docs"

CARD, TEXT, MUTED = "#1b1f24", "#e6edf3", "#9198a1"
BODY, CAP, CAP_EDGE = "#2b3036", "#3a4048", "#4d5560"
KEY, GAP = 46, 8
FONT = "font-family='-apple-system,Segoe UI,Helvetica,Arial,sans-serif'"


class Example:
    """A herdr state for AgentPad.colors()."""

    def __init__(self, agents=(), active=None, status=None, all_agents=None):
        self.agents = list(agents)
        self.all_agents = list(all_agents if all_agents is not None else agents)
        self.active = active
        self.status = status or {}


def led_colors(state, layer=1, brightness=100, all_workspaces=False):
    pad = daemon.AgentPad(None)
    pad.layer, pad.brightness, pad.all_workspaces = layer, brightness, all_workspaces
    return pad.colors(state)


def glow(rgb):
    """Screen fill and opacity for an LED colour: hue at full strength, brightness as opacity."""
    peak = max(rgb)
    if peak == 0:
        return None, 0
    fill = "#{:02x}{:02x}{:02x}".format(*(round(c * 255 / peak) for c in rgb))
    return fill, max(0.18, (peak / 255) ** 0.5)  # LEDs look brighter than their values


def text(x, y, s, size=13, color=TEXT, anchor="start", weight="normal"):
    return (
        f"<text x='{x}' y='{y}' {FONT} font-size='{size}' fill='{color}' "
        f"text-anchor='{anchor}' font-weight='{weight}'>{escape(s)}</text>"
    )


def key(x, y, rgb, label="", flash=False, size=KEY):
    parts = [
        f"<rect x='{x}' y='{y}' width='{size}' height='{size}' rx='7' fill='{CAP}' stroke='{CAP_EDGE}'/>"
    ]
    fill, opacity = glow(rgb)
    if fill:
        anim = (
            "<animate attributeName='opacity' values='1;0' dur='1s' calcMode='discrete' repeatCount='indefinite'/>"
            if flash
            else ""
        )
        parts.append(
            f"<rect x='{x + 3}' y='{y + 3}' width='{size - 6}' height='{size - 6}' rx='5' "
            f"fill='{fill}' opacity='{opacity:.2f}'>{anim}</rect>"
        )
    lines = label.split("\n")
    for i, line in enumerate(lines):
        dark = fill and opacity > 0.6 and fill not in ("#0000ff", "#003cff")
        color = "#111" if dark else TEXT
        ty = y + size / 2 + 4 + (i - (len(lines) - 1) / 2) * 12
        parts.append(text(x + size / 2, ty, line, size=11, color=color, anchor="middle"))
    return "".join(parts)


def hexcolor(rgb, lighten=0.0):
    """Hex for rgb, optionally mixed towards white (for text on the dark card)."""
    return "#{:02x}{:02x}{:02x}".format(*(round(c + (255 - c) * lighten) for c in rgb))


def knob(cx, cy, label="", layer=None, selected=False):
    """A knob; with a layer, its ring shows that layer's mode colour, glowing when selected."""
    ring = hexcolor(daemon.LAYER_COLORS[layer]) if layer else CAP_EDGE
    out = ""
    if selected:
        out += f"<circle cx='{cx}' cy='{cy}' r='26' fill='{ring}' opacity='0.35'/>"
    out += (
        f"<circle cx='{cx}' cy='{cy}' r='19' fill='{CAP}' stroke='{ring}' stroke-width='3'/>"
        f"<circle cx='{cx}' cy='{cy}' r='12' fill='{BODY}' stroke='{CAP_EDGE}'/>"
        f"<line x1='{cx}' y1='{cy - 12}' x2='{cx}' y2='{cy - 5}' stroke='{MUTED}' stroke-width='2'/>"
    )
    if label:
        out += text(cx, cy + 34, label, size=11, color=MUTED, anchor="middle")
    if layer:
        name_color = hexcolor(daemon.LAYER_COLORS[layer], lighten=0.35)
        out += text(cx, cy + 48, daemon.LAYER_NAMES[layer], 11, name_color, "middle", "bold")
    return out


def pill(x, y, label, layer):
    """A badge in a layer's mode colour."""
    w = 12 + 7.2 * len(label)
    return (
        f"<rect x='{x}' y='{y}' width='{w:.0f}' height='22' rx='11' fill='{hexcolor(daemon.LAYER_COLORS[layer])}'/>"
        + text(x + w / 2, y + 15, label, size=12, color="#ffffff", anchor="middle", weight="bold")
    )


def pad(x, y, colors, labels=(), flashing=(), knobs=True, layer=1):
    """A schematic pad: knobs above a 4x4 grid; colors and labels by physical position."""
    grid_y = y + (92 if knobs else 14)
    w = 4 * KEY + 3 * GAP + 28
    h = grid_y - y + 4 * KEY + 3 * GAP + 14
    parts = [f"<rect x='{x}' y='{y}' width='{w}' height='{h}' rx='14' fill='{BODY}'/>"]
    if knobs:
        for n in range(3):
            parts.append(knob(x + w / 2 + (n - 1) * 62, y + 30, f"knob {n + 1}", n + 1, n + 1 == layer))
    labels = list(labels) + [""] * (16 - len(labels))
    for pos in range(16):
        kx = x + 14 + (pos % 4) * (KEY + GAP)
        ky = grid_y + (pos // 4) * (KEY + GAP)
        parts.append(key(kx, ky, colors[pos], labels[pos], pos in flashing))
    return "".join(parts), w, h


def svg(name, width, height, body):
    DOCS.mkdir(exist_ok=True)
    (DOCS / name).write_text(
        f"<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}' "
        f"viewBox='0 0 {width} {height}'>"
        f"<rect width='{width}' height='{height}' rx='12' fill='{CARD}'/>{body}</svg>\n"
    )


def example_agents():
    status = {"a1": "working", "a2": "done", "a3": "blocked", "a4": "idle", "a5": "working"}
    return Example(agents=list(status), active="a1", status=status)


def layout():
    colors = led_colors(example_agents())
    labels = [str(n) for n in range(1, 13)] + ["1", "2", "3", "esc"]
    body, w, h = pad(24, 56, colors, labels, flashing={2})
    body += text(24, 56 + h + 24, "Bottom row now:", size=12, color=MUTED)
    body += pill(124, 56 + h + 9, f"{daemon.LAYER_NAMES[1]} mode", 1)
    notes = [
        ("Knob 1", "turn: previous / next workspace"),
        ("", "press: layer 1, Claude mode · ×3: all workspaces"),
        ("Knob 2", "turn: previous / next agent in the workspace"),
        ("", "press: layer 2, Codex mode"),
        ("Knob 3", "turn: brightness ±5% (5–100%)"),
        ("", "press: layer 3, Kiro mode"),
        ("Rows 1–3", "agent keys 1–12: press to focus that agent;"),
        ("", "colour shows its status"),
        ("Bottom row", "answer keys, typed into the active agent;"),
        ("", "colour shows the layer"),
    ]
    tx = 24 + w + 32
    for i, (head, line) in enumerate(notes):
        ty = 92 + i * 27 + (i // 2) * 10
        if head:
            body += text(tx, ty, head, weight="bold")
        body += text(tx + 92, ty, line, color=MUTED if not head else TEXT)
    body += text(24, 34, "Controls (layer 1, agents 1–5 in the focused workspace)", size=15, weight="bold")
    svg("layout.svg", 760, 428, body)


def layers():
    body = text(
        24, 34, "Layers: press a knob to pick its layer; the bottom row changes", size=15, weight="bold"
    )
    for i, layer in enumerate((1, 2, 3)):
        y = 58 + i * 70
        colors = led_colors(Example(), layer=layer)[12:]
        body += knob(46, y + 23, layer=layer, selected=True).split("<text")[0]  # ring only
        body += pill(80, y + 2, f"{daemon.LAYER_NAMES[layer]} mode", layer)
        body += text(80, y + 42, f"layer {layer} · press knob {layer}", size=12, color=MUTED)
        for k, (rgb, name) in enumerate(zip(colors, daemon.BOTTOM_KEYS[layer], strict=True)):
            body += key(220 + k * (KEY + GAP), y, rgb, name or "–")
        sent = ", ".join(n for n in daemon.BOTTOM_KEYS[layer] if n)
        body += text(460, y + 28, f"sends {sent}", size=12, color=MUTED)
    body += text(
        24, 280, "Unmapped keys (–) stay lit in the layer colour and do nothing.", size=12, color=MUTED
    )
    svg("layers.svg", 620, 300, body)


def status():
    cols = [
        ("working", "working", "Working"),
        ("blocked", "blocked", "Waiting for you"),
        ("done", "done", "Done, not viewed"),
        ("idle", "idle", "Idle / unknown"),
        (None, None, "No agent"),
    ]
    body = text(24, 34, "Agent keys: colour = herdr status", size=15, weight="bold")
    for c, (_, _status, name) in enumerate(cols):
        body += text(190 + c * 110 + KEY / 2, 70, name, size=12, color=MUTED, anchor="middle")
    for r, (row, is_active) in enumerate((("Active agent", True), ("Other agents", False))):
        y = 90 + r * 70
        body += text(24, y + 20, row, weight="bold")
        body += text(
            24, y + 38, "full brightness" if is_active else f"1/{daemon.INACTIVE_DIM} brightness", 12, MUTED
        )
        for c, (agent, st, _) in enumerate(cols):
            state = Example(agents=[agent] if agent else [], active=agent if is_active else None)
            state.status = {agent: st} if agent else {}
            rgb = led_colors(state)[0] if agent else daemon.OFF
            body += key(190 + c * 110, y, rgb, flash=st == "blocked")
    body += text(24, 250, "Waiting for you flashes red. Done turns idle once you focus the agent.", 12, MUTED)
    svg("status.svg", 760, 270, body)


def modes():
    w1 = ["w1:p1", "w1:p2"]
    w2 = ["w2:p1", "w2:p2", "w2:p3"]
    st = {"w1:p1": "working", "w1:p2": "idle", "w2:p1": "blocked", "w2:p2": "done", "w2:p3": "working"}
    state = Example(agents=w1, all_agents=w1 + w2, active="w1:p1", status=st)
    body = text(
        24, 34, "Agent key modes (knob 1 pressed 3 times within a second toggles)", size=15, weight="bold"
    )
    body += text(24, 56, "Example: workspace w1 (focused) has 2 agents, w2 has 3", size=12, color=MUTED)
    for i, (title, everywhere) in enumerate(
        (("Focused workspace (default)", False), ("All workspaces", True))
    ):
        x = 24 + i * 300
        agents = state.all_agents if everywhere else state.agents
        labels = [a.replace(":", "\n") for a in agents]
        colors = led_colors(state, all_workspaces=everywhere)
        flashing = {agents.index("w2:p1")} if "w2:p1" in agents else set()
        part, _, _ = pad(x, 100, colors, labels, flashing, knobs=False)
        body += text(x, 88, title, weight="bold") + part
    body += text(
        24, 358, "In all-workspaces mode a key jumps straight to its agent, switching workspace.", 12, MUTED
    )
    svg("modes.svg", 600, 378, body)


def brightness():
    body = text(24, 34, "Knob 3: brightness scales every LED", size=15, weight="bold")
    for i, pct in enumerate((100, 50, 5)):
        x = 24 + i * 250
        part, _, _ = pad(x, 70, led_colors(example_agents(), brightness=pct), knobs=False)
        body += text(x, 60, f"{pct}%", weight="bold") + part
    body += text(24, 330, "5% per click, 5–100%, remembered in ~/.local/state/agentpad-brightness", 12, MUTED)
    svg("brightness.svg", 760, 350, body)


def box(x, y, w, h, title, lines=(), accent=CAP_EDGE):
    out = f"<rect x='{x}' y='{y}' width='{w}' height='{h}' rx='10' fill='{BODY}' stroke='{accent}'/>"
    out += text(x + 12, y + 22, title, weight="bold")
    for i, line in enumerate(lines):
        out += text(x + 12, y + 42 + i * 17, line, size=12, color=MUTED)
    return out


def arrow(x1, y1, x2, y2, label="", both=False):
    marker = "marker-end='url(#a)'" + (" marker-start='url(#a)'" if both else "")
    out = f"<line x1='{x1}' y1='{y1}' x2='{x2}' y2='{y2}' stroke='{MUTED}' stroke-width='1.5' {marker}/>"
    if label:
        out += text((x1 + x2) / 2 + 6, (y1 + y2) / 2 - 6, label, size=11, color=MUTED)
    return out


def architecture():
    defs = (
        "<defs><marker id='a' viewBox='0 0 10 10' refX='9' refY='5' markerWidth='7' markerHeight='7' "
        f"orient='auto-start-reverse'><path d='M0,0 L10,5 L0,10 z' fill='{MUTED}'/></marker></defs>"
    )
    body = defs + text(24, 34, "How it fits together", size=15, weight="bold")
    body += box(
        24, 60, 200, 96, "SIDE-KEYBOARD", ["16 keys, 3 knobs", "profile 5: F13–F24 codes", "per-key RGB LEDs"]
    )
    body += box(300, 52, 210, 58, "hidraw interface 1", ["key reports (read raw)"])
    body += box(300, 124, 210, 58, "hidraw interface 2", ["config: profile, codes, LEDs"])
    body += box(300, 196, 210, 58, "input devices", ["grabbed, events discarded"])
    agentpad_lines = ["systemd user service", "layers, modes,", "brightness, LED frames"]
    body += box(586, 52, 160, 116, "agentpad", agentpad_lines, "#6e7681")
    body += box(586, 262, 160, 78, "herdr", ["workspaces, tabs,", "panes, agents"])
    body += arrow(224, 90, 298, 81)
    body += arrow(224, 120, 298, 153, both=True)
    body += arrow(224, 146, 298, 225)
    body += arrow(510, 81, 584, 100)
    body += arrow(584, 140, 512, 153)
    body += arrow(584, 160, 512, 205, "grab")
    body += arrow(666, 168, 666, 260, both=True)
    for i, line in enumerate(("socket API:", "focus, send keys;", "status every 0.25 s")):
        body += text(656, 208 + i * 15, line, size=11, color=MUTED, anchor="end")
    svg("architecture.svg", 770, 356, body)


def main():
    daemon.time.monotonic = lambda: 0.0  # render "waiting" keys in the lit half of their flash
    for draw in (layout, layers, status, modes, brightness, architecture):
        draw()
    print("wrote", ", ".join(sorted(p.name for p in DOCS.glob("*.svg"))))


if __name__ == "__main__":
    main()
