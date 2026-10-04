import json
import os
import shutil
import socketserver
import tempfile
import threading

import pytest

from agentpad import daemon


class FakeHerdr:
    """Just enough of herdr's socket API: two workspaces, agents, focus, send_keys."""

    def __init__(self):
        self.panes = {"w1": ["w1:p1", "w1:p2", "w1:p3"], "w2": ["w2:p1"]}
        self.workspace = "w1"
        self.focused = {"w1": "w1:p1", "w2": "w2:p1"}
        self.status = {}
        self.calls = []

    def handle(self, method, params):
        self.calls.append((method, params))
        if method == "workspace.list":
            return {
                "workspaces": [
                    {"workspace_id": w, "number": n, "focused": w == self.workspace}
                    for n, w in enumerate(self.panes, 1)
                ]
            }
        if method == "agent.list":
            return {
                "agents": [
                    {
                        "pane_id": p,
                        "workspace_id": w,
                        "tab_id": f"{w}:t1",
                        "focused": w == self.workspace and p == self.focused[w],
                        "agent_status": self.status.get(p, "idle"),
                    }
                    for w, panes in self.panes.items()
                    for p in panes
                ]
            }
        if method == "workspace.focus":
            self.workspace = params["workspace_id"]
            return {}
        if method == "agent.focus":
            pane = params["target"]
            self.workspace = pane.split(":")[0]
            self.focused[self.workspace] = pane
            return {}
        if method == "pane.send_keys":
            return {}
        raise KeyError(method)


@pytest.fixture
def herdr(monkeypatch):
    fake = FakeHerdr()

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            req = json.loads(self.rfile.readline())
            try:
                reply = {"id": req["id"], "result": fake.handle(req["method"], req["params"])}
            except KeyError as e:
                reply = {"id": req["id"], "error": {"code": "unknown_method", "message": str(e)}}
            self.wfile.write((json.dumps(reply) + "\n").encode())

    tmp = tempfile.mkdtemp(prefix="agentpad-")  # short path: unix sockets max out at 108 bytes
    path = os.path.join(tmp, "herdr.sock")
    server = socketserver.ThreadingUnixStreamServer(path, Handler)
    threading.Thread(target=server.serve_forever, args=(0.01,), daemon=True).start()
    monkeypatch.setattr(daemon, "HERDR_SOCK", path)
    yield fake
    server.shutdown()
    server.server_close()
    shutil.rmtree(tmp)


@pytest.fixture(autouse=True)
def brightness_file(tmp_path, monkeypatch):
    path = tmp_path / "brightness"
    monkeypatch.setattr(daemon, "BRIGHTNESS_FILE", str(path))
    return path
