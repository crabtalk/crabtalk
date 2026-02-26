//! Protocol type serialization tests.

use walrus_protocol::{ClientMessage, PROTOCOL_VERSION, ServerMessage};

#[test]
fn protocol_version() {
    assert_eq!(PROTOCOL_VERSION, "0.1");
}

#[test]
fn client_authenticate_roundtrip() {
    let msg = ClientMessage::Authenticate {
        token: "secret".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ClientMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ClientMessage::Authenticate { token } if token == "secret"));
}

#[test]
fn client_send_roundtrip() {
    let msg = ClientMessage::Send {
        agent: "alpha".into(),
        content: "hello".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ClientMessage = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(back, ClientMessage::Send { agent, content } if agent == "alpha" && content == "hello")
    );
}

#[test]
fn client_stream_roundtrip() {
    let msg = ClientMessage::Stream {
        agent: "beta".into(),
        content: "world".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ClientMessage = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(back, ClientMessage::Stream { agent, content } if agent == "beta" && content == "world")
    );
}

#[test]
fn client_clear_session_roundtrip() {
    let msg = ClientMessage::ClearSession {
        agent: "gamma".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ClientMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ClientMessage::ClearSession { agent } if agent == "gamma"));
}

#[test]
fn client_ping_roundtrip() {
    let msg = ClientMessage::Ping;
    let json = serde_json::to_string(&msg).unwrap();
    let back: ClientMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ClientMessage::Ping));
}

#[test]
fn server_authenticated_roundtrip() {
    let msg = ServerMessage::Authenticated {
        session_id: "sess-123".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(back, ServerMessage::Authenticated { session_id } if session_id == "sess-123")
    );
}

#[test]
fn server_response_roundtrip() {
    let msg = ServerMessage::Response {
        agent: "alpha".into(),
        content: "reply".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(back, ServerMessage::Response { agent, content } if agent == "alpha" && content == "reply")
    );
}

#[test]
fn server_stream_start_roundtrip() {
    let msg = ServerMessage::StreamStart {
        agent: "beta".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ServerMessage::StreamStart { agent } if agent == "beta"));
}

#[test]
fn server_stream_chunk_roundtrip() {
    let msg = ServerMessage::StreamChunk {
        content: "chunk-data".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ServerMessage::StreamChunk { content } if content == "chunk-data"));
}

#[test]
fn server_stream_end_roundtrip() {
    let msg = ServerMessage::StreamEnd {
        agent: "beta".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ServerMessage::StreamEnd { agent } if agent == "beta"));
}

#[test]
fn server_session_cleared_roundtrip() {
    let msg = ServerMessage::SessionCleared {
        agent: "gamma".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ServerMessage::SessionCleared { agent } if agent == "gamma"));
}

#[test]
fn server_error_roundtrip() {
    let msg = ServerMessage::Error {
        code: 404,
        message: "not found".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(back, ServerMessage::Error { code, message } if code == 404 && message == "not found")
    );
}

#[test]
fn server_pong_roundtrip() {
    let msg = ServerMessage::Pong;
    let json = serde_json::to_string(&msg).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ServerMessage::Pong));
}

#[test]
fn tagged_json_format() {
    let msg = ClientMessage::Send {
        agent: "alpha".into(),
        content: "hello".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["type"], "send");
    assert_eq!(value["agent"], "alpha");
    assert_eq!(value["content"], "hello");
}
