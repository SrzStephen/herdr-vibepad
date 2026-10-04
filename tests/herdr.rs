mod support;
use std::path::Path;

#[test]
fn returns_the_result_field_on_success() {
    let fake = support::FakeHerdr::start(|_method, _params| Ok(serde_json::json!({"ok": true})));
    let result = herdr_vibepad::herdr::call(&fake.sock_path, "ping", serde_json::json!({}));
    assert_eq!(result, Some(serde_json::json!({"ok": true})));
}

#[test]
fn returns_none_on_an_error_reply() {
    let fake =
        support::FakeHerdr::start(|_method, _params| Err(serde_json::json!({"message": "boom"})));
    assert_eq!(
        herdr_vibepad::herdr::call(&fake.sock_path, "ping", serde_json::json!({})),
        None
    );
}

#[test]
fn returns_none_when_nothing_is_listening() {
    let result = herdr_vibepad::herdr::call(
        Path::new("/nonexistent/herdr.sock"),
        "ping",
        serde_json::json!({}),
    );
    assert_eq!(result, None);
}

type Served = (tempfile::TempDir, std::path::PathBuf);

fn serve_once(reply: impl FnOnce(std::os::unix::net::UnixStream) + Send + 'static) -> Served {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("herdr.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            reply(stream);
        }
    });
    (dir, path)
}

fn drain_request(stream: &std::os::unix::net::UnixStream) {
    use std::io::BufRead;
    let mut line = String::new();
    let _ = std::io::BufReader::new(stream).read_line(&mut line);
}

#[test]
fn returns_none_on_malformed_json_reply() {
    use std::io::Write;
    let (_dir, path) = serve_once(|mut s| {
        drain_request(&s);
        let _ = s.write_all(b"{not json\n");
    });
    assert_eq!(
        herdr_vibepad::herdr::call(&path, "ping", serde_json::json!({})),
        None
    );
}

#[test]
fn returns_none_when_server_closes_without_replying() {
    let (_dir, path) = serve_once(|s| {
        drain_request(&s);
    });
    assert_eq!(
        herdr_vibepad::herdr::call(&path, "ping", serde_json::json!({})),
        None
    );
}

#[test]
fn returns_none_on_a_non_object_reply() {
    use std::io::Write;
    let (_dir, path) = serve_once(|mut s| {
        drain_request(&s);
        let _ = s.write_all(b"[]\n");
    });
    assert_eq!(
        herdr_vibepad::herdr::call(&path, "ping", serde_json::json!({})),
        None
    );
}

#[test]
fn returns_none_within_the_timeout_when_server_never_replies() {
    let (_dir, path) = serve_once(|s| {
        drain_request(&s);
        std::thread::sleep(std::time::Duration::from_secs(6));
    });
    let start = std::time::Instant::now();
    assert_eq!(
        herdr_vibepad::herdr::call(&path, "ping", serde_json::json!({})),
        None
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}
