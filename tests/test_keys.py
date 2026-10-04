import pytest

from agentpad import keys


@pytest.mark.parametrize(
    ("text", "entry"),
    [
        ("a", [keys.TYPE_KEY, 0, 0x04, 0]),
        ("ctrl+shift+t", [keys.TYPE_KEY, 0x03, 0x17, 0]),
        ("f13", [keys.TYPE_KEY, 0, 0x68, 0]),
        ("f24", [keys.TYPE_KEY, 0, 0x73, 0]),
        ("volup", [keys.TYPE_CONSUMER, 0xE9, 0, 0]),
        ("profileswitch", [keys.TYPE_FUNCTION, 0x13, 0, 0]),
    ],
)
def test_parse_key(text, entry):
    assert keys.parse_key(text) == entry


@pytest.mark.parametrize("text", ["a", "ctrl+shift+t", "shift+f18", "esc", "mute", "profileswitch"])
def test_describe_round_trips(text):
    assert keys.describe(keys.parse_key(text)) == text


@pytest.mark.parametrize(("text", "slot"), [("0", 0), ("15", 15), ("knob1.press", 16), ("knob3.left", 24)])
def test_slots(text, slot):
    assert keys.parse_slot(text) == slot
    assert keys.slot_name(slot) == (f"key {text}" if text.isdigit() else text)


def test_bad_key_exits():
    with pytest.raises(SystemExit):
        keys.parse_key("hyper+q")
