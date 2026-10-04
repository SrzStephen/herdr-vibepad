//! JSON-RPC client for herdr's Unix-socket API.
//!
//! Ports `daemon.py`'s `herdr()` function.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

/// Call `method` with `params` over the herdr socket at `sock_path`, returning
/// the reply's `"result"` field, or `None` on any connect/timeout/parse error
/// or an `"error"` reply (both are logged via [`crate::log`]).
pub fn call(
    sock_path: &Path,
    method: &str,
    params: serde_json::Value,
) -> Option<serde_json::Value> {
    match try_call(sock_path, method, params) {
        Ok(reply) => {
            if let Some(error) = reply.get("error") {
                crate::log(&format!("herdr {method}: {error}"), false);
                return None;
            }
            reply.get("result").cloned()
        }
        Err(e) => {
            crate::log(&format!("herdr {method}: {e}"), true);
            None
        }
    }
}

fn try_call(
    sock_path: &Path,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let stream = UnixStream::connect(sock_path).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;

    let request = serde_json::json!({"id": "herdr-vibepad", "method": method, "params": params});
    let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    line.push(b'\n');

    let mut write_stream = stream.try_clone().map_err(|e| e.to_string())?;
    write_stream.write_all(&line).map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    let mut reader = BufReader::new(stream);
    reader
        .read_until(b'\n', &mut buf)
        .map_err(|e| e.to_string())?;

    serde_json::from_slice(&buf).map_err(|e| e.to_string())
}
