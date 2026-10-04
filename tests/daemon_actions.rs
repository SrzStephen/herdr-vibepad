mod support;
use agentpad::daemon::{self, AgentPad, State};
use agentpad::keys;
use std::path::Path;

fn knob(n: usize, part: &str) -> usize {
    keys::NUM_KEYS + 3 * (n - 1) + keys::KNOB_PARTS.iter().position(|p| *p == part).unwrap()
}
fn key_at(pos: usize) -> usize {
    (0..keys::NUM_KEYS)
        .find(|&i| daemon::position(i) == pos)
        .unwrap()
}
fn press(pad: &mut AgentPad, slot: usize, sock: &Path, now: f64) -> State {
    pad.press(slot, &State::fetch(sock), now);
    State::fetch(sock)
}

#[test]
fn knob2_steps_through_agents_and_wraps() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    let mut active = vec![];
    for _ in 0..3 {
        active.push(press(&mut pad, knob(2, "right"), &fake.sock_path, 0.0).active);
    }
    assert_eq!(
        active,
        [
            Some("w1:p2".into()),
            Some("w1:p3".into()),
            Some("w1:p1".into())
        ]
    );
    assert_eq!(
        press(&mut pad, knob(2, "left"), &fake.sock_path, 0.0).active,
        Some("w1:p3".into())
    );
}

#[test]
fn knob1_steps_through_workspaces_and_wraps() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    assert_eq!(
        press(&mut pad, knob(1, "right"), &fake.sock_path, 0.0).workspace,
        Some("w2".into())
    );
    assert_eq!(
        press(&mut pad, knob(1, "right"), &fake.sock_path, 0.0).workspace,
        Some("w1".into())
    );
    assert_eq!(
        press(&mut pad, knob(1, "left"), &fake.sock_path, 0.0).workspace,
        Some("w2".into())
    );
}

#[test]
fn agent_key_focuses_that_agent() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    assert_eq!(
        press(&mut pad, key_at(2), &fake.sock_path, 0.0).active,
        Some("w1:p3".into())
    );
    assert_eq!(
        press(&mut pad, key_at(0), &fake.sock_path, 0.0).active,
        Some("w1:p1".into())
    );
}

#[test]
fn agent_key_without_agent_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    press(&mut pad, key_at(7), &fake.sock_path, 0.0);
    assert!(!fake.calls().iter().any(|(m, _)| m == "agent.focus"));
}

#[test]
fn knob_press_selects_layer() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    for n in [3, 2, 1] {
        press(&mut pad, knob(n, "press"), &fake.sock_path, 0.0);
        assert_eq!(pad.layer, n as u8);
    }
}

#[test]
fn bottom_row_sends_layer_keys_to_active_agent() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    for layer in 1..=3usize {
        press(&mut pad, knob(layer, "press"), &fake.sock_path, 0.0);
        for pos in 12..16 {
            press(&mut pad, key_at(pos), &fake.sock_path, 0.0);
        }
    }
    let sent: Vec<(String, String)> = fake
        .calls()
        .into_iter()
        .filter(|(m, _)| m == "pane.send_keys")
        .map(|(_, p)| {
            (
                p["pane_id"].as_str().unwrap().to_string(),
                p["keys"][0].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let expected: Vec<&str> = [1, 2, 3]
        .iter()
        .flat_map(|&l| daemon::BOTTOM_KEYS[l - 1].iter().flatten().copied())
        .collect();
    assert_eq!(expected[..4], ["1", "2", "3", "esc"]);
    assert_eq!(
        sent,
        expected
            .iter()
            .map(|k| ("w1:p1".to_string(), k.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn knob3_changes_brightness_within_limits_and_remembers() {
    let fake = support::two_workspace_herdr();
    let dir = tempfile::tempdir().unwrap();
    let brightness_file = dir.path().join("brightness");
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), brightness_file.clone());
    assert_eq!(pad.brightness, 100);
    press(&mut pad, knob(3, "right"), &fake.sock_path, 0.0);
    assert_eq!(pad.brightness, 100);
    for _ in 0..40 {
        press(&mut pad, knob(3, "left"), &fake.sock_path, 0.0);
    }
    assert_eq!(pad.brightness, daemon::BRIGHTNESS_MIN);
    assert_eq!(pad.brightness, 5);
    press(&mut pad, knob(3, "right"), &fake.sock_path, 0.0);
    assert_eq!(pad.brightness, 10);
    assert_eq!(
        std::fs::read_to_string(&brightness_file).unwrap().trim(),
        "10"
    );
    assert_eq!(
        AgentPad::new(None, fake.sock_path.clone(), brightness_file).brightness,
        10
    );
}

#[test]
fn no_herdr_means_empty_state() {
    let st = State::fetch(Path::new("/nonexistent/herdr.sock"));
    assert!(st.workspaces.is_empty() && st.agents.is_empty() && st.active.is_none());
}

fn triple_press_knob1(pad: &mut AgentPad, sock: &Path, gap: f64, start: f64) {
    for i in 0..3 {
        press(pad, knob(1, "press"), sock, start + i as f64 * gap);
    }
}

#[test]
fn three_quick_knob1_presses_toggle_all_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 100.0);
    assert!(pad.all_workspaces && pad.layer == 1);
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 200.0);
    assert!(!pad.all_workspaces);
}

#[test]
fn slow_or_double_knob1_presses_do_not_toggle() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    triple_press_knob1(&mut pad, &fake.sock_path, 0.6, 300.0); // 1.2s from first to third
    assert!(!pad.all_workspaces);
    pad.knob1_presses.clear();
    press(&mut pad, knob(1, "press"), &fake.sock_path, 400.0);
    press(&mut pad, knob(1, "press"), &fake.sock_path, 400.1);
    assert!(!pad.all_workspaces);
}

#[test]
fn all_workspaces_key_jumps_to_agent_in_other_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    assert_eq!(
        press(&mut pad, key_at(3), &fake.sock_path, 0.0).active,
        Some("w1:p1".into())
    );
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 500.0);
    let st = press(&mut pad, key_at(3), &fake.sock_path, 0.0);
    assert_eq!(
        (st.workspace, st.active),
        (Some("w2".into()), Some("w2:p1".into()))
    );
    assert_eq!(
        press(&mut pad, key_at(0), &fake.sock_path, 0.0).active,
        Some("w1:p1".into())
    );
}

#[test]
fn all_workspaces_mode_lights_every_agent() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::two_workspace_herdr();
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    let lit = |pad: &AgentPad, st: &State| {
        pad.colors(st, 0.0)[..12]
            .iter()
            .filter(|&&c| c != daemon::OFF)
            .count()
    };
    assert_eq!(lit(&pad, &State::fetch(&fake.sock_path)), 3);
    triple_press_knob1(&mut pad, &fake.sock_path, 0.1, 600.0);
    assert_eq!(lit(&pad, &State::fetch(&fake.sock_path)), 4);
}

fn state_with(agents: &[&str], active: Option<&str>, status: &[(&str, &str)]) -> State {
    State {
        workspaces: vec![],
        workspace: None,
        all_agents: agents.iter().map(|s| s.to_string()).collect(),
        agents: agents.iter().map(|s| s.to_string()).collect(),
        status: status
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        active: active.map(str::to_string),
    }
}

#[test]
fn bottom_row_is_layer_colour_at_20_percent() {
    let dir = tempfile::tempdir().unwrap();
    for layer in 1..=3u8 {
        let pad = AgentPad::new(None, dir.path().join("unused"), dir.path().join("unused"));
        let mut pad = pad;
        pad.layer = layer;
        let (r, g, b) = daemon::LAYER_COLORS[(layer - 1) as usize];
        let expected = [
            daemon::round_half_even(r as f64 * 0.2) as u8,
            daemon::round_half_even(g as f64 * 0.2) as u8,
            daemon::round_half_even(b as f64 * 0.2) as u8,
        ];
        for i in 12..16 {
            assert_eq!(
                pad.colors(&state_with(&[], None, &[]), 0.0)[i],
                (expected[0], expected[1], expected[2])
            );
        }
    }
}

#[test]
fn agent_keys_show_status_active_bright_others_dimmed() {
    let dir = tempfile::tempdir().unwrap();
    let pad = AgentPad::new(None, dir.path().join("unused"), dir.path().join("unused"));
    let st = state_with(
        &["a", "b", "c", "d"],
        Some("b"),
        &[
            ("a", "working"),
            ("b", "done"),
            ("c", "idle"),
            ("d", "unknown"),
        ],
    );
    let out = pad.colors(&st, 0.0);
    let dim = |c: (u8, u8, u8)| {
        (
            (c.0 as u32 / daemon::INACTIVE_DIM) as u8,
            (c.1 as u32 / daemon::INACTIVE_DIM) as u8,
            (c.2 as u32 / daemon::INACTIVE_DIM) as u8,
        )
    };
    assert_eq!(out[0], dim(daemon::status_color("working")));
    assert_eq!(out[1], daemon::status_color("done"));
    assert_eq!(out[2], dim(daemon::status_color("idle")));
    assert_eq!(out[3], out[2]); // unknown shows as idle
    for c in &out[4..12] {
        assert_eq!(*c, daemon::OFF);
    }
}

#[test]
fn blocked_agent_flashes() {
    let dir = tempfile::tempdir().unwrap();
    let pad = AgentPad::new(None, dir.path().join("unused"), dir.path().join("unused"));
    let st = state_with(&["a"], Some("a"), &[("a", "blocked")]);
    assert_eq!(pad.colors(&st, 0.1)[0], daemon::status_color("blocked"));
    assert_eq!(pad.colors(&st, daemon::FLASH + 0.1)[0], daemon::OFF);
}

#[test]
fn brightness_scales_everything() {
    let dir = tempfile::tempdir().unwrap();
    let mut pad = AgentPad::new(None, dir.path().join("unused"), dir.path().join("unused"));
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

#[test]
fn round_half_even_matches_pythons_banker_rounding() {
    for (input, expected) in [
        (0.5, 0),
        (1.5, 2),
        (2.5, 2),
        (3.5, 4),
        (25.5, 26),
        (127.5, 128),
    ] {
        assert_eq!(daemon::round_half_even(input), expected);
    }
}

#[test]
fn knob_turn_does_nothing_with_no_agents_or_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let fake = support::FakeHerdr::start(|method, _| {
        Ok(match method {
            "workspace.list" => serde_json::json!({"workspaces": []}),
            "agent.list" => serde_json::json!({"agents": []}),
            _ => serde_json::json!({}),
        })
    });
    let mut pad = AgentPad::new(None, fake.sock_path.clone(), dir.path().join("unused"));
    press(&mut pad, knob(1, "right"), &fake.sock_path, 0.0); // must not panic
    press(&mut pad, knob(2, "right"), &fake.sock_path, 0.0); // must not panic
}
