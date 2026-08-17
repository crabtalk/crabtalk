//! Conversions between the generated messages — wrapping a payload in its
//! envelope, and unwrapping the envelope again.

use crate::{client_message, server_message};

impl From<crate::SendMsg> for crate::ClientMessage {
    fn from(msg: crate::SendMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Send(msg)),
        }
    }
}

impl From<crate::StreamMsg> for crate::ClientMessage {
    fn from(msg: crate::StreamMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Stream(msg)),
        }
    }
}

impl From<crate::SendResponse> for crate::ServerMessage {
    fn from(r: crate::SendResponse) -> Self {
        Self {
            msg: Some(server_message::Msg::Response(r)),
        }
    }
}

impl From<crate::StreamEvent> for crate::ServerMessage {
    fn from(e: crate::StreamEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::Stream(e)),
        }
    }
}

impl From<crate::AgentEventMsg> for crate::ServerMessage {
    fn from(e: crate::AgentEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::AgentEvent(e)),
        }
    }
}

impl From<crate::McpEventMsg> for crate::ServerMessage {
    fn from(e: crate::McpEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::McpEvent(e)),
        }
    }
}

impl From<crate::ConversationHistory> for crate::ServerMessage {
    fn from(h: crate::ConversationHistory) -> Self {
        Self {
            msg: Some(server_message::Msg::ConversationHistory(h)),
        }
    }
}

#[cfg(feature = "client")]
mod message {
    use crate::server_message;

    impl crate::ServerMessage {
        /// Convert a `ServerMessage` to an `anyhow::Error`.
        pub fn error_or_unexpected(self) -> anyhow::Error {
            match self.msg {
                Some(server_message::Msg::Error(e)) => {
                    anyhow::anyhow!("server error ({}): {}", e.code, e.message)
                }
                _ => anyhow::anyhow!("unexpected response: {:?}", self.msg),
            }
        }
    }

    impl TryFrom<crate::ServerMessage> for crate::SendResponse {
        type Error = anyhow::Error;
        fn try_from(msg: crate::ServerMessage) -> anyhow::Result<Self> {
            match msg.msg {
                Some(server_message::Msg::Response(r)) => Ok(r),
                _ => Err(msg.error_or_unexpected()),
            }
        }
    }

    impl TryFrom<crate::ServerMessage> for crate::stream_event::Event {
        type Error = anyhow::Error;
        fn try_from(msg: crate::ServerMessage) -> anyhow::Result<Self> {
            match msg.msg {
                Some(server_message::Msg::Stream(e)) => {
                    e.event.ok_or_else(|| anyhow::anyhow!("empty stream event"))
                }
                _ => Err(msg.error_or_unexpected()),
            }
        }
    }
}
