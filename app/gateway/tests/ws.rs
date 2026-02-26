//! WebSocket protocol tests.

use protocol::{ClientMessage, ServerMessage};

#[test]
fn client_send_serializes() {
    let msg = ClientMessage::Send {
        agent: "assistant".into(),
        content: "hello".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"send\""));
    assert!(json.contains("\"agent\":\"assistant\""));
}

#[test]
fn server_response_serializes() {
    let msg = ServerMessage::Response {
        agent: "assistant".into(),
        content: "hi there".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"response\""));
}

#[test]
fn server_error_serializes() {
    let msg = ServerMessage::Error {
        code: 401,
        message: "unauthorized".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"code\":401"));
}

#[test]
fn ping_pong_roundtrip() {
    let ping = serde_json::to_string(&ClientMessage::Ping).unwrap();
    let parsed: ClientMessage = serde_json::from_str(&ping).unwrap();
    assert!(matches!(parsed, ClientMessage::Ping));

    let pong = serde_json::to_string(&ServerMessage::Pong).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&pong).unwrap();
    assert!(matches!(parsed, ServerMessage::Pong));
}
