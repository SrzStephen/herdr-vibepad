import pytest

from agentpad import daemon


class State:
    def __init__(self, agents=(), active=None, status=None):
        self.agents = list(agents)
        self.active = active
        self.status = status or {}


def colors(layer=1, brightness=100, now=0.0, monkeypatch=None, **state):
    pad = daemon.AgentPad(None)
    pad.layer, pad.brightness = layer, brightness
    if monkeypatch:
        monkeypatch.setattr(daemon.time, "monotonic", lambda: now)
    return pad.colors(State(**state))


@pytest.mark.parametrize("layer", [1, 2, 3])
def test_bottom_row_is_layer_colour_at_20_percent(layer):
    rgb = daemon.LAYER_COLORS[layer]
    # Unmapped keys too: any dimmer and purple's red channel vanishes, so it looks blue.
    assert colors(layer)[12:] == [tuple(round(c * 0.2) for c in rgb)] * 4


def test_agent_keys_show_status_active_bright_others_dimmed():
    status = {"a": "working", "b": "done", "c": "idle", "d": "unknown"}
    out = colors(agents="abcd", active="b", status=status)
    dim = daemon.INACTIVE_DIM
    assert out[0] == tuple(c // dim for c in daemon.STATUS_COLORS["working"])
    assert out[1] == daemon.STATUS_COLORS["done"]
    assert out[2] == tuple(c // dim for c in daemon.STATUS_COLORS["idle"])
    assert out[3] == out[2]  # unknown shows as idle
    assert out[4:12] == [daemon.OFF] * 8


def test_blocked_agent_flashes(monkeypatch):
    kw = {"agents": "a", "active": "a", "status": {"a": "blocked"}, "monkeypatch": monkeypatch}
    assert colors(now=0.1, **kw)[0] == daemon.STATUS_COLORS["blocked"]
    assert colors(now=daemon.FLASH + 0.1, **kw)[0] == daemon.OFF


def test_brightness_scales_everything():
    full = colors(agents="a", active="a")
    half = colors(agents="a", active="a", brightness=50)
    assert half == [tuple(round(c / 2) for c in rgb) for rgb in full]
