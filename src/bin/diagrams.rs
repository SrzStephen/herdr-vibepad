//! Generate the README's SVG diagrams in docs/ from the daemon's own layout
//! and colours.
//!
//! Run with `cargo run --release --bin diagrams` after changing colours,
//! layers or the key layout. Ported from `scripts/diagrams.py`, which stays
//! in the tree as read-only reference material until the Python source is
//! retired.
//!
//! Dev-only: never installed by the .deb, and (like its Python original) has
//! no automated tests.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use agentpad::daemon;

const CARD: &str = "#1b1f24";
const TEXT: &str = "#e6edf3";
const MUTED: &str = "#9198a1";
const BODY: &str = "#2b3036";
const CAP: &str = "#3a4048";
const CAP_EDGE: &str = "#4d5560";
const KEY: i64 = 46;
const GAP: i64 = 8;
const FONT: &str = "font-family='-apple-system,Segoe UI,Helvetica,Arial,sans-serif'";

fn docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs")
}

/// `daemon`'s layer tables are 0-indexed (layer 1-3 -> index 0-2); diagrams.py's
/// source used 1-indexed dicts, so every lookup here re-does that shift.
fn layer_color(layer: u8) -> (u8, u8, u8) {
    daemon::LAYER_COLORS[(layer - 1) as usize]
}

fn layer_name(layer: u8) -> &'static str {
    daemon::LAYER_NAMES[(layer - 1) as usize]
}

fn bottom_keys(layer: u8) -> [Option<&'static str>; 4] {
    daemon::BOTTOM_KEYS[(layer - 1) as usize]
}

/// Python's `str(x)` for a float: like Rust's shortest round-trip `Display`,
/// but always keeps a `.0` for a whole number (`80.0`, not `80`).
fn pf(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A herdr state for `AgentPad::colors()`.
fn led_colors(
    state: &daemon::State,
    layer: u8,
    brightness: i32,
    all_workspaces: bool,
) -> [(u8, u8, u8); 16] {
    let mut pad = daemon::AgentPad::new(None, PathBuf::new(), PathBuf::new());
    pad.layer = layer;
    pad.brightness = brightness;
    pad.all_workspaces = all_workspaces;
    pad.colors(state, 0.0)
}

/// Screen fill and opacity for an LED colour: hue at full strength,
/// brightness as opacity. `None` when the colour is off.
fn glow(rgb: (u8, u8, u8)) -> Option<(String, f64)> {
    let peak = rgb.0.max(rgb.1).max(rgb.2);
    if peak == 0 {
        return None;
    }
    let hexpart = |c: u8| daemon::round_half_even(c as f64 * 255.0 / peak as f64) as u8;
    let fill = format!(
        "#{:02x}{:02x}{:02x}",
        hexpart(rgb.0),
        hexpart(rgb.1),
        hexpart(rgb.2)
    );
    // LEDs look brighter than their values.
    let opacity = (peak as f64 / 255.0).powf(0.5).max(0.18);
    Some((fill, opacity))
}

fn text(x: &str, y: &str, s: &str, size: i64, color: &str, anchor: &str, weight: &str) -> String {
    format!(
        "<text x='{x}' y='{y}' {FONT} font-size='{size}' fill='{color}' text-anchor='{anchor}' font-weight='{weight}'>{}</text>",
        escape_xml(s)
    )
}

fn key(x: i64, y: i64, rgb: (u8, u8, u8), label: &str, flash: bool) -> String {
    let size = KEY;
    let mut out = format!("<rect x='{x}' y='{y}' width='{size}' height='{size}' rx='7' fill='{CAP}' stroke='{CAP_EDGE}'/>");
    let glowed = glow(rgb);
    if let Some((fill, opacity)) = &glowed {
        let anim = if flash {
            "<animate attributeName='opacity' values='1;0' dur='1s' calcMode='discrete' repeatCount='indefinite'/>"
        } else {
            ""
        };
        out += &format!(
            "<rect x='{}' y='{}' width='{}' height='{}' rx='5' fill='{fill}' opacity='{opacity:.2}'>{anim}</rect>",
            x + 3,
            y + 3,
            size - 6,
            size - 6
        );
    }
    let lines: Vec<&str> = label.split('\n').collect();
    let dark = glowed
        .as_ref()
        .map(|(fill, opacity)| {
            *opacity > 0.6 && fill.as_str() != "#0000ff" && fill.as_str() != "#003cff"
        })
        .unwrap_or(false);
    let n = lines.len();
    for (i, line) in lines.iter().enumerate() {
        let color = if dark { "#111" } else { TEXT };
        let tx = pf(x as f64 + size as f64 / 2.0);
        let ty =
            pf(y as f64 + size as f64 / 2.0 + 4.0 + (i as f64 - (n as f64 - 1.0) / 2.0) * 12.0);
        out += &text(&tx, &ty, line, 11, color, "middle", "normal");
    }
    out
}

fn hexcolor(rgb: (u8, u8, u8), lighten: f64) -> String {
    let mix = |c: u8| daemon::round_half_even(c as f64 + (255.0 - c as f64) * lighten) as u8;
    format!("#{:02x}{:02x}{:02x}", mix(rgb.0), mix(rgb.1), mix(rgb.2))
}

/// A knob; with a layer, its ring shows that layer's mode colour, glowing
/// when selected. `cx` is pre-formatted (it's sometimes a Python float,
/// sometimes a Python int, at different call sites); `cy` is always an int
/// in this file.
fn knob(cx: &str, cy: i64, label: &str, layer: Option<u8>, selected: bool) -> String {
    let ring = match layer {
        Some(l) => hexcolor(layer_color(l), 0.0),
        None => CAP_EDGE.to_string(),
    };
    let mut out = String::new();
    if selected {
        out += &format!("<circle cx='{cx}' cy='{cy}' r='26' fill='{ring}' opacity='0.35'/>");
    }
    out += &format!(
        "<circle cx='{cx}' cy='{cy}' r='19' fill='{CAP}' stroke='{ring}' stroke-width='3'/><circle cx='{cx}' cy='{cy}' r='12' fill='{BODY}' stroke='{CAP_EDGE}'/><line x1='{cx}' y1='{}' x2='{cx}' y2='{}' stroke='{MUTED}' stroke-width='2'/>",
        cy - 12,
        cy - 5
    );
    if !label.is_empty() {
        out += &text(
            cx,
            &(cy + 34).to_string(),
            label,
            11,
            MUTED,
            "middle",
            "normal",
        );
    }
    if let Some(l) = layer {
        let name_color = hexcolor(layer_color(l), 0.35);
        out += &text(
            cx,
            &(cy + 48).to_string(),
            layer_name(l),
            11,
            &name_color,
            "middle",
            "bold",
        );
    }
    out
}

/// A badge in a layer's mode colour.
fn pill(x: i64, y: i64, label: &str, layer: u8) -> String {
    let w = 12.0 + 7.2 * label.chars().count() as f64;
    let w_rounded = daemon::round_half_even(w);
    let fill = hexcolor(layer_color(layer), 0.0);
    let rect =
        format!("<rect x='{x}' y='{y}' width='{w_rounded}' height='22' rx='11' fill='{fill}'/>");
    let tx = pf(x as f64 + w / 2.0);
    rect + &text(
        &tx,
        &(y + 15).to_string(),
        label,
        12,
        "#ffffff",
        "middle",
        "bold",
    )
}

/// A schematic pad: knobs above a 4x4 grid; colors and labels by physical
/// position.
fn pad(
    x: i64,
    y: i64,
    colors: &[(u8, u8, u8)],
    labels: &[&str],
    flashing: &HashSet<usize>,
    knobs: bool,
    layer: u8,
) -> (String, i64, i64) {
    let grid_y = y + if knobs { 92 } else { 14 };
    let w: i64 = 4 * KEY + 3 * GAP + 28;
    let h: i64 = grid_y - y + 4 * KEY + 3 * GAP + 14;
    let mut out = format!("<rect x='{x}' y='{y}' width='{w}' height='{h}' rx='14' fill='{BODY}'/>");
    if knobs {
        for n in 0..3i64 {
            let cx = pf(x as f64 + w as f64 / 2.0 + (n - 1) as f64 * 62.0);
            out += &knob(
                &cx,
                y + 30,
                &format!("knob {}", n + 1),
                Some((n + 1) as u8),
                (n + 1) as u8 == layer,
            );
        }
    }
    let mut lbls: Vec<&str> = labels.to_vec();
    lbls.resize(16, "");
    for pos in 0..16usize {
        let kx = x + 14 + (pos % 4) as i64 * (KEY + GAP);
        let ky = grid_y + (pos / 4) as i64 * (KEY + GAP);
        out += &key(kx, ky, colors[pos], lbls[pos], flashing.contains(&pos));
    }
    (out, w, h)
}

fn svg(name: &str, width: i64, height: i64, body: &str) {
    let docs = docs_dir();
    std::fs::create_dir_all(&docs).expect("create docs/");
    let content = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}' viewBox='0 0 {width} {height}'><rect width='{width}' height='{height}' rx='12' fill='{CARD}'/>{body}</svg>\n"
    );
    std::fs::write(docs.join(name), content).expect("write svg");
}

fn example_agents() -> daemon::State {
    let status: HashMap<String, String> = [
        ("a1", "working"),
        ("a2", "done"),
        ("a3", "blocked"),
        ("a4", "idle"),
        ("a5", "working"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let agents: Vec<String> = ["a1", "a2", "a3", "a4", "a5"]
        .into_iter()
        .map(String::from)
        .collect();
    daemon::State {
        workspaces: vec![],
        workspace: None,
        all_agents: agents.clone(),
        agents,
        status,
        active: Some("a1".to_string()),
    }
}

fn layout() {
    let state = example_agents();
    let colors = led_colors(&state, 1, 100, false);
    let labels_owned: Vec<String> = (1..=12)
        .map(|n: i32| n.to_string())
        .chain(["1", "2", "3", "esc"].into_iter().map(String::from))
        .collect();
    let labels: Vec<&str> = labels_owned.iter().map(|s| s.as_str()).collect();
    let flashing: HashSet<usize> = [2].into_iter().collect();
    let (mut body, w, h) = pad(24, 56, &colors, &labels, &flashing, true, 1);
    body += &text(
        &24.to_string(),
        &(56 + h + 24).to_string(),
        "Bottom row now:",
        12,
        MUTED,
        "start",
        "normal",
    );
    body += &pill(124, 56 + h + 9, &format!("{} mode", layer_name(1)), 1);
    let notes: [(&str, &str); 10] = [
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
    ];
    let tx = 24 + w + 32;
    for (i, (head, line)) in notes.iter().enumerate() {
        let ty = 92 + i as i64 * 27 + (i as i64 / 2) * 10;
        if !head.is_empty() {
            body += &text(
                &tx.to_string(),
                &ty.to_string(),
                head,
                13,
                TEXT,
                "start",
                "bold",
            );
        }
        let color = if head.is_empty() { MUTED } else { TEXT };
        body += &text(
            &(tx + 92).to_string(),
            &ty.to_string(),
            line,
            13,
            color,
            "start",
            "normal",
        );
    }
    body += &text(
        &24.to_string(),
        &34.to_string(),
        "Controls (layer 1, agents 1–5 in the focused workspace)",
        15,
        TEXT,
        "start",
        "bold",
    );
    svg("layout.svg", 760, 428, &body);
}

fn layers() {
    let mut body = text(
        &24.to_string(),
        &34.to_string(),
        "Layers: press a knob to pick its layer; the bottom row changes",
        15,
        TEXT,
        "start",
        "bold",
    );
    let empty_state = daemon::State {
        workspaces: vec![],
        workspace: None,
        all_agents: vec![],
        agents: vec![],
        status: HashMap::new(),
        active: None,
    };
    for (i, layer) in [1u8, 2, 3].into_iter().enumerate() {
        let y: i64 = 58 + i as i64 * 70;
        let colors_full = led_colors(&empty_state, layer, 100, false);
        let colors = &colors_full[12..16];
        let knob_str = knob(&46.to_string(), y + 23, "", Some(layer), true);
        let ring_only = knob_str.split("<text").next().unwrap();
        body += ring_only;
        body += &pill(80, y + 2, &format!("{} mode", layer_name(layer)), layer);
        body += &text(
            &80.to_string(),
            &(y + 42).to_string(),
            &format!("layer {layer} · press knob {layer}"),
            12,
            MUTED,
            "start",
            "normal",
        );
        let bk = bottom_keys(layer);
        for (k, (rgb, name)) in colors.iter().zip(bk.iter()).enumerate() {
            let label = name.unwrap_or("–");
            body += &key(220 + k as i64 * (KEY + GAP), y, *rgb, label, false);
        }
        let sent = bk.iter().filter_map(|n| *n).collect::<Vec<_>>().join(", ");
        body += &text(
            &460.to_string(),
            &(y + 28).to_string(),
            &format!("sends {sent}"),
            12,
            MUTED,
            "start",
            "normal",
        );
    }
    body += &text(
        &24.to_string(),
        &280.to_string(),
        "Unmapped keys (–) stay lit in the layer colour and do nothing.",
        12,
        MUTED,
        "start",
        "normal",
    );
    svg("layers.svg", 620, 300, &body);
}

fn status() {
    let cols: [(Option<&str>, Option<&str>, &str); 5] = [
        (Some("working"), Some("working"), "Working"),
        (Some("blocked"), Some("blocked"), "Waiting for you"),
        (Some("done"), Some("done"), "Done, not viewed"),
        (Some("idle"), Some("idle"), "Idle / unknown"),
        (None, None, "No agent"),
    ];
    let mut body = text(
        &24.to_string(),
        &34.to_string(),
        "Agent keys: colour = herdr status",
        15,
        TEXT,
        "start",
        "bold",
    );
    for (c, &(_, _st, name)) in cols.iter().enumerate() {
        let x = pf(190.0 + c as f64 * 110.0 + KEY as f64 / 2.0);
        body += &text(&x, &70.to_string(), name, 12, MUTED, "middle", "normal");
    }
    for (r, (row, is_active)) in [("Active agent", true), ("Other agents", false)]
        .into_iter()
        .enumerate()
    {
        let y: i64 = 90 + r as i64 * 70;
        body += &text(
            &24.to_string(),
            &(y + 20).to_string(),
            row,
            13,
            TEXT,
            "start",
            "bold",
        );
        let brightness_note = if is_active {
            "full brightness".to_string()
        } else {
            format!("1/{} brightness", daemon::INACTIVE_DIM)
        };
        body += &text(
            &24.to_string(),
            &(y + 38).to_string(),
            &brightness_note,
            12,
            MUTED,
            "start",
            "normal",
        );
        for (c, &(agent, st, _name)) in cols.iter().enumerate() {
            let agent_vec = agent.map(|a| vec![a.to_string()]).unwrap_or_default();
            let status: HashMap<String, String> = match (agent, st) {
                (Some(a), Some(s)) => [(a.to_string(), s.to_string())].into_iter().collect(),
                _ => HashMap::new(),
            };
            let state = daemon::State {
                workspaces: vec![],
                workspace: None,
                all_agents: agent_vec.clone(),
                agents: agent_vec,
                status,
                active: if is_active {
                    agent.map(String::from)
                } else {
                    None
                },
            };
            let rgb = if agent.is_some() {
                led_colors(&state, 1, 100, false)[0]
            } else {
                daemon::OFF
            };
            body += &key(190 + c as i64 * 110, y, rgb, "", st == Some("blocked"));
        }
    }
    body += &text(
        &24.to_string(),
        &250.to_string(),
        "Waiting for you flashes red. Done turns idle once you focus the agent.",
        12,
        MUTED,
        "start",
        "normal",
    );
    svg("status.svg", 760, 270, &body);
}

fn modes() {
    let w1: Vec<String> = ["w1:p1", "w1:p2"].into_iter().map(String::from).collect();
    let w2: Vec<String> = ["w2:p1", "w2:p2", "w2:p3"]
        .into_iter()
        .map(String::from)
        .collect();
    let status: HashMap<String, String> = [
        ("w1:p1", "working"),
        ("w1:p2", "idle"),
        ("w2:p1", "blocked"),
        ("w2:p2", "done"),
        ("w2:p3", "working"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let mut all_agents = w1.clone();
    all_agents.extend(w2.clone());
    let state = daemon::State {
        workspaces: vec![],
        workspace: None,
        all_agents,
        agents: w1,
        status,
        active: Some("w1:p1".to_string()),
    };
    let mut body = text(
        &24.to_string(),
        &34.to_string(),
        "Agent key modes (knob 1 pressed 3 times within a second toggles)",
        15,
        TEXT,
        "start",
        "bold",
    );
    body += &text(
        &24.to_string(),
        &56.to_string(),
        "Example: workspace w1 (focused) has 2 agents, w2 has 3",
        12,
        MUTED,
        "start",
        "normal",
    );
    for (i, (title, everywhere)) in [
        ("Focused workspace (default)", false),
        ("All workspaces", true),
    ]
    .into_iter()
    .enumerate()
    {
        let x: i64 = 24 + i as i64 * 300;
        let agents: &Vec<String> = if everywhere {
            &state.all_agents
        } else {
            &state.agents
        };
        let labels_owned: Vec<String> = agents.iter().map(|a| a.replace(':', "\n")).collect();
        let labels: Vec<&str> = labels_owned.iter().map(|s| s.as_str()).collect();
        let colors = led_colors(&state, 1, 100, everywhere);
        let flashing: HashSet<usize> = agents
            .iter()
            .position(|a| a == "w2:p1")
            .into_iter()
            .collect();
        let (part, _w, _h) = pad(x, 100, &colors, &labels, &flashing, false, 1);
        body += &text(
            &x.to_string(),
            &88.to_string(),
            title,
            13,
            TEXT,
            "start",
            "bold",
        );
        body += &part;
    }
    body += &text(
        &24.to_string(),
        &358.to_string(),
        "In all-workspaces mode a key jumps straight to its agent, switching workspace.",
        12,
        MUTED,
        "start",
        "normal",
    );
    svg("modes.svg", 600, 378, &body);
}

fn brightness() {
    let mut body = text(
        &24.to_string(),
        &34.to_string(),
        "Knob 3: brightness scales every LED",
        15,
        TEXT,
        "start",
        "bold",
    );
    let empty: [&str; 0] = [];
    for (i, pct) in [100i32, 50, 5].into_iter().enumerate() {
        let x: i64 = 24 + i as i64 * 250;
        let state = example_agents();
        let colors = led_colors(&state, 1, pct, false);
        let (part, _w, _h) = pad(x, 70, &colors, &empty, &HashSet::new(), false, 1);
        body += &text(
            &x.to_string(),
            &60.to_string(),
            &format!("{pct}%"),
            13,
            TEXT,
            "start",
            "bold",
        );
        body += &part;
    }
    body += &text(
        &24.to_string(),
        &330.to_string(),
        "5% per click, 5–100%, remembered in ~/.local/state/agentpad-brightness",
        12,
        MUTED,
        "start",
        "normal",
    );
    svg("brightness.svg", 760, 350, &body);
}

fn box_(x: i64, y: i64, w: i64, h: i64, title: &str, lines: &[&str], accent: &str) -> String {
    let mut out = format!(
        "<rect x='{x}' y='{y}' width='{w}' height='{h}' rx='10' fill='{BODY}' stroke='{accent}'/>"
    );
    out += &text(
        &(x + 12).to_string(),
        &(y + 22).to_string(),
        title,
        13,
        TEXT,
        "start",
        "bold",
    );
    for (i, line) in lines.iter().enumerate() {
        out += &text(
            &(x + 12).to_string(),
            &(y + 42 + i as i64 * 17).to_string(),
            line,
            12,
            MUTED,
            "start",
            "normal",
        );
    }
    out
}

fn arrow(x1: i64, y1: i64, x2: i64, y2: i64, label: &str, both: bool) -> String {
    let marker = if both {
        "marker-end='url(#a)' marker-start='url(#a)'".to_string()
    } else {
        "marker-end='url(#a)'".to_string()
    };
    let mut out = format!("<line x1='{x1}' y1='{y1}' x2='{x2}' y2='{y2}' stroke='{MUTED}' stroke-width='1.5' {marker}/>");
    if !label.is_empty() {
        let tx = pf((x1 + x2) as f64 / 2.0 + 6.0);
        let ty = pf((y1 + y2) as f64 / 2.0 - 6.0);
        out += &text(&tx, &ty, label, 11, MUTED, "start", "normal");
    }
    out
}

fn architecture() {
    let defs = format!(
        "<defs><marker id='a' viewBox='0 0 10 10' refX='9' refY='5' markerWidth='7' markerHeight='7' orient='auto-start-reverse'><path d='M0,0 L10,5 L0,10 z' fill='{MUTED}'/></marker></defs>"
    );
    let mut body = defs
        + &text(
            &24.to_string(),
            &34.to_string(),
            "How it fits together",
            15,
            TEXT,
            "start",
            "bold",
        );
    body += &box_(
        24,
        60,
        200,
        96,
        "SIDE-KEYBOARD",
        &[
            "16 keys, 3 knobs",
            "profile 5: F13–F24 codes",
            "per-key RGB LEDs",
        ],
        CAP_EDGE,
    );
    body += &box_(
        300,
        52,
        210,
        58,
        "hidraw interface 1",
        &["key reports (read raw)"],
        CAP_EDGE,
    );
    body += &box_(
        300,
        124,
        210,
        58,
        "hidraw interface 2",
        &["config: profile, codes, LEDs"],
        CAP_EDGE,
    );
    body += &box_(
        300,
        196,
        210,
        58,
        "input devices",
        &["grabbed, events discarded"],
        CAP_EDGE,
    );
    body += &box_(
        586,
        52,
        160,
        116,
        "agentpad",
        &[
            "systemd user service",
            "layers, modes,",
            "brightness, LED frames",
        ],
        "#6e7681",
    );
    body += &box_(
        586,
        262,
        160,
        78,
        "herdr",
        &["workspaces, tabs,", "panes, agents"],
        CAP_EDGE,
    );
    body += &arrow(224, 90, 298, 81, "", false);
    body += &arrow(224, 120, 298, 153, "", true);
    body += &arrow(224, 146, 298, 225, "", false);
    body += &arrow(510, 81, 584, 100, "", false);
    body += &arrow(584, 140, 512, 153, "", false);
    body += &arrow(584, 160, 512, 205, "grab", false);
    body += &arrow(666, 168, 666, 260, "", true);
    for (i, line) in ["socket API:", "focus, send keys;", "status every 0.25 s"]
        .into_iter()
        .enumerate()
    {
        body += &text(
            &656.to_string(),
            &(208 + i as i64 * 15).to_string(),
            line,
            11,
            MUTED,
            "end",
            "normal",
        );
    }
    svg("architecture.svg", 770, 356, &body);
}

fn main() {
    layout();
    layers();
    status();
    modes();
    brightness();
    architecture();
    let mut names: Vec<String> = std::fs::read_dir(docs_dir())
        .expect("read docs/")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.ends_with(".svg").then_some(name)
        })
        .collect();
    names.sort();
    println!("wrote {}", names.join(", "));
}
