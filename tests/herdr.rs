mod support;
use std::path::Path;

#[test]
fn returns_the_result_field_on_success() {
    let fake = support::FakeHerdr::start(|_method, _params| Ok(serde_json::json!({"ok": true})));
    let result = agentpad::herdr::call(&fake.sock_path, "ping", serde_json::json!({}));
    assert_eq!(result, Some(serde_json::json!({"ok": true})));
}

#[test]
fn returns_none_on_an_error_reply() {
    let fake =
        support::FakeHerdr::start(|_method, _params| Err(serde_json::json!({"message": "boom"})));
    assert_eq!(
        agentpad::herdr::call(&fake.sock_path, "ping", serde_json::json!({})),
        None
    );
}

#[test]
fn returns_none_when_nothing_is_listening() {
    let result = agentpad::herdr::call(
        Path::new("/nonexistent/herdr.sock"),
        "ping",
        serde_json::json!({}),
    );
    assert_eq!(result, None);
}
