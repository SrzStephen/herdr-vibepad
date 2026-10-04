from agentpad import daemon, keys


def knob(n, part):
    return keys.NUM_KEYS + 3 * (n - 1) + keys.KNOB_PARTS.index(part)


def key_at(pos):
    """The pad slot of the key at a physical position (left to right, top to bottom)."""
    return next(i for i in range(keys.NUM_KEYS) if daemon.position(i) == pos)


def press(pad, slot):
    pad.press(slot, daemon.State())
    return daemon.State()


def test_knob2_steps_through_agents_and_wraps(herdr):
    pad = daemon.AgentPad(None)
    assert [press(pad, knob(2, "right")).active for _ in range(3)] == ["w1:p2", "w1:p3", "w1:p1"]
    assert press(pad, knob(2, "left")).active == "w1:p3"


def test_knob1_steps_through_workspaces_and_wraps(herdr):
    pad = daemon.AgentPad(None)
    assert press(pad, knob(1, "right")).workspace == "w2"
    assert press(pad, knob(1, "right")).workspace == "w1"
    assert press(pad, knob(1, "left")).workspace == "w2"


def test_agent_key_focuses_that_agent(herdr):
    pad = daemon.AgentPad(None)
    assert press(pad, key_at(2)).active == "w1:p3"
    assert press(pad, key_at(0)).active == "w1:p1"


def test_agent_key_without_agent_does_nothing(herdr):
    pad = daemon.AgentPad(None)
    press(pad, key_at(7))
    assert not [c for c in herdr.calls if c[0] == "agent.focus"]


def test_knob_press_selects_layer(herdr):
    pad = daemon.AgentPad(None)
    for n in (3, 2, 1):
        press(pad, knob(n, "press"))
        assert pad.layer == n


def sent_keys(herdr):
    return [(c[1]["pane_id"], c[1]["keys"][0]) for c in herdr.calls if c[0] == "pane.send_keys"]


def test_bottom_row_sends_layer_keys_to_active_agent(herdr):
    pad = daemon.AgentPad(None)
    for layer in (1, 2, 3):
        press(pad, knob(layer, "press"))
        for pos in range(12, 16):
            press(pad, key_at(pos))
    expected = [k for layer in (1, 2, 3) for k in daemon.BOTTOM_KEYS[layer] if k]
    assert sent_keys(herdr) == [("w1:p1", k) for k in expected]
    assert expected[:4] == ["1", "2", "3", "esc"]


def test_knob3_changes_brightness_within_limits_and_remembers(herdr, brightness_file):
    pad = daemon.AgentPad(None)
    assert pad.brightness == 100
    press(pad, knob(3, "right"))
    assert pad.brightness == 100
    for _ in range(40):
        press(pad, knob(3, "left"))
    assert pad.brightness == daemon.BRIGHTNESS_MIN == 5
    press(pad, knob(3, "right"))
    assert pad.brightness == 10
    assert brightness_file.read_text().strip() == "10"
    assert daemon.AgentPad(None).brightness == 10


def test_no_herdr_means_empty_state(monkeypatch):
    monkeypatch.setattr(daemon, "HERDR_SOCK", "/nonexistent/herdr.sock")
    st = daemon.State()
    assert (st.workspaces, st.agents, st.active) == ([], [], None)


def triple_press_knob1(pad, monkeypatch, gap=0.1, start=100.0):
    for i in range(3):
        monkeypatch.setattr(daemon.time, "monotonic", lambda t=start + i * gap: t)
        press(pad, knob(1, "press"))


def test_three_quick_knob1_presses_toggle_all_workspaces(herdr, monkeypatch):
    pad = daemon.AgentPad(None)
    triple_press_knob1(pad, monkeypatch)
    assert pad.all_workspaces and pad.layer == 1
    triple_press_knob1(pad, monkeypatch, start=200.0)
    assert not pad.all_workspaces


def test_slow_or_double_knob1_presses_do_not_toggle(herdr, monkeypatch):
    pad = daemon.AgentPad(None)
    triple_press_knob1(pad, monkeypatch, gap=0.6)  # 1.2 s from first to third
    assert not pad.all_workspaces
    pad.knob1_presses = []
    for t in (300.0, 300.1):
        monkeypatch.setattr(daemon.time, "monotonic", lambda t=t: t)
        press(pad, knob(1, "press"))
    assert not pad.all_workspaces


def test_all_workspaces_key_jumps_to_agent_in_other_workspace(herdr, monkeypatch):
    pad = daemon.AgentPad(None)
    assert press(pad, key_at(3)).active == "w1:p1"  # focused workspace has 3 agents: key 4 is empty
    triple_press_knob1(pad, monkeypatch)
    st = press(pad, key_at(3))  # w1:p1 w1:p2 w1:p3 w2:p1
    assert (st.workspace, st.active) == ("w2", "w2:p1")
    assert press(pad, key_at(0)).active == "w1:p1"


def test_all_workspaces_mode_lights_every_agent(herdr, monkeypatch):
    pad = daemon.AgentPad(None)
    st = daemon.State()
    lit = lambda: sum(rgb != daemon.OFF for rgb in pad.colors(st)[:12])  # noqa: E731
    assert lit() == 3
    triple_press_knob1(pad, monkeypatch)
    assert lit() == 4
