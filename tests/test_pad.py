import os

import pytest

from agentpad import daemon, keys


def test_positions_cover_every_key_once():
    assert sorted(daemon.position(i) for i in range(keys.NUM_KEYS)) == list(range(keys.NUM_KEYS))


@pytest.mark.parametrize(
    ("index", "where"),
    [
        (3, 0),  # pressing top-left sent slot 3
        (12, 15),  # pressing bottom-right sent slot 12
        (0, 12),  # LED 0 lit bottom-left
        (15, 3),  # LED 15 lit top-right
    ],
)
def test_position_matches_what_the_pad_did(index, where):
    assert daemon.position(index) == where


def test_every_slot_sends_a_distinct_code():
    assert len(daemon.SLOT_ENTRIES) == keys.NUM_SLOTS
    assert len(daemon.CODE_TO_SLOT) == keys.NUM_SLOTS


def feed(reports):
    """Run hidraw key reports through Pad.events and return the slots pressed."""
    pad = daemon.Pad.__new__(daemon.Pad)
    hid_r, hid_w = os.pipe()  # command replies: stays empty
    rep_r, rep_w = os.pipe()
    os.set_blocking(rep_r, False)  # like the real hidraw node
    pad.hid, pad.reports, pad.inputs, pad.down = hid_r, rep_r, [], []
    slots = []
    for hexreport in reports:
        os.write(rep_w, bytes.fromhex(hexreport))
        slots += pad.events(0)
    for fd in (hid_r, hid_w, rep_r, rep_w):
        os.close(fd)
    return slots


def test_decodes_reports_captured_from_the_pad():
    reports = [
        "0200 6c00 0000 0000",  # shift+f17: knob1 press
        "0000 0000 0000 0000",
        "0200 6f00 0000 0000",  # shift+f20: knob2 press
        "0200 6f72 0000 0000",  # shift+f23 while f20 is still held: knob3 press
        "0000 6f72 0000 0000",
        "0000 0000 0000 0000",
        "0200 6d00 0000 0000",  # shift+f18: knob1 right
        "0000 0000 0000 0000",
        "0100 6800 0000 0000",  # ctrl+f13: knob3 left
        "0000 0000 0000 0000",
        "0000 6b00 0000 0000",  # f16: key slot 3 (top left)
        "0000 0000 0000 0000",
        "0200 6800 0000 0000",  # shift+f13: key slot 12 (bottom right)
    ]
    assert feed(reports) == [16, 19, 22, 17, 24, 3, 12]


def test_held_key_counts_once():
    assert feed(["0000 6800 0000 0000"] * 3) == [0]
