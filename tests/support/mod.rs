//! Test-only harness: a real Unix-socket JSON-RPC fake, standing in for
//! herdr the way `tests/conftest.py`'s `FakeHerdr` (built on a real
//! threaded `socketserver.ThreadingUnixStreamServer`) does for the Python
//! test suite. No mocking framework — this project deliberately tests
//! through real OS primitives.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

type Handler = dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, serde_json::Value>
    + Send
    + Sync
    + 'static;

/// A real Unix-socket JSON-RPC server, backed by a `UnixListener` bound
/// inside a short-lived `TempDir` (kept alive so the socket file isn't
/// cleaned up from under the listener; unix socket paths max out at 108
/// bytes, same constraint `tests/conftest.py` notes).
pub struct FakeHerdr {
    pub sock_path: PathBuf,
    _tempdir: tempfile::TempDir,
    _handle: JoinHandle<()>,
    calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl FakeHerdr {
    /// Start the fake server on a background thread. `handler` is called
    /// once per request with `(method, params)` and its `Ok`/`Err` becomes
    /// the reply's `result`/`error` field.
    pub fn start<F>(handler: F) -> FakeHerdr
    where
        F: Fn(&str, serde_json::Value) -> Result<serde_json::Value, serde_json::Value>
            + Send
            + Sync
            + 'static,
    {
        let tempdir = tempfile::Builder::new()
            .prefix("herdr-vibepad-")
            .tempdir()
            .expect("create tempdir for fake herdr socket");
        let sock_path = tempdir.path().join("herdr.sock");
        let listener = UnixListener::bind(&sock_path).expect("bind fake herdr socket");

        let calls: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_thread = Arc::clone(&calls);
        let handler: Arc<Handler> = Arc::new(handler);

        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let handler = Arc::clone(&handler);
                let calls = Arc::clone(&calls_thread);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let Ok(req) = serde_json::from_str::<serde_json::Value>(&line) else {
                        return;
                    };
                    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
                    let method = req
                        .get("method")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let params = req
                        .get("params")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);

                    calls
                        .lock()
                        .expect("calls lock poisoned")
                        .push((method.clone(), params.clone()));

                    let reply = match handler(&method, params) {
                        Ok(result) => serde_json::json!({"id": id, "result": result}),
                        Err(error) => serde_json::json!({"id": id, "error": error}),
                    };
                    let mut out = serde_json::to_vec(&reply).expect("serialize reply");
                    out.push(b'\n');
                    let _ = stream.write_all(&out);
                });
            }
        });

        FakeHerdr {
            sock_path,
            _tempdir: tempdir,
            _handle: handle,
            calls,
        }
    }

    /// The `(method, params)` of every request handled so far, in order.
    pub fn calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.lock().expect("calls lock poisoned").clone()
    }
}

/// Internal state of [`two_workspace_herdr`]'s fake: two workspaces (`w1`
/// with panes p1-p3, `w2` with pane p1), which workspace is focused, and
/// which pane is focused within each workspace.
struct TwoWorkspaceState {
    panes: Vec<(&'static str, Vec<&'static str>)>,
    workspace: String,
    focused: HashMap<String, String>,
}

/// Just enough of herdr's socket API to drive `VibePad`: two workspaces,
/// agents, focus, send_keys. Ports `tests/conftest.py`'s `FakeHerdr` class.
pub fn two_workspace_herdr() -> FakeHerdr {
    let state = Arc::new(Mutex::new(TwoWorkspaceState {
        panes: vec![
            ("w1", vec!["w1:p1", "w1:p2", "w1:p3"]),
            ("w2", vec!["w2:p1"]),
        ],
        workspace: "w1".to_string(),
        focused: HashMap::from([
            ("w1".to_string(), "w1:p1".to_string()),
            ("w2".to_string(), "w2:p1".to_string()),
        ]),
    }));

    FakeHerdr::start(move |method, params| {
        let mut st = state.lock().expect("fake herdr state lock poisoned");
        match method {
            "workspace.list" => {
                let workspaces: Vec<serde_json::Value> = st
                    .panes
                    .iter()
                    .enumerate()
                    .map(|(i, (w, _))| {
                        serde_json::json!({
                            "workspace_id": w,
                            "number": i + 1,
                            "focused": *w == st.workspace,
                        })
                    })
                    .collect();
                Ok(serde_json::json!({"workspaces": workspaces}))
            }
            "agent.list" => {
                let mut agents = Vec::new();
                for (w, panes) in &st.panes {
                    for p in panes {
                        let focused = *w == st.workspace
                            && st.focused.get(*w).map(String::as_str) == Some(*p);
                        agents.push(serde_json::json!({
                            "pane_id": p,
                            "workspace_id": w,
                            "tab_id": format!("{w}:t1"),
                            "focused": focused,
                            "agent_status": "idle",
                        }));
                    }
                }
                Ok(serde_json::json!({"agents": agents}))
            }
            "workspace.focus" => {
                if let Some(wid) = params.get("workspace_id").and_then(|v| v.as_str()) {
                    st.workspace = wid.to_string();
                }
                Ok(serde_json::json!({}))
            }
            "agent.focus" => {
                if let Some(target) = params.get("target").and_then(|v| v.as_str()) {
                    let ws = target.split(':').next().unwrap_or("").to_string();
                    st.focused.insert(ws.clone(), target.to_string());
                    st.workspace = ws;
                }
                Ok(serde_json::json!({}))
            }
            "pane.send_keys" => Ok(serde_json::json!({})),
            other => Err(serde_json::json!({"code": "unknown_method", "message": other})),
        }
    })
}
