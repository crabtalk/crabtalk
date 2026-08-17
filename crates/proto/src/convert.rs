//! Conversions between the generated messages — wrapping a payload in its
//! envelope, and unwrapping the envelope again.

use crate::{
    AgentEventMsg, ClientMessage, ConversationHistory, McpEventMsg, SendMsg, SendResponse,
    ServerMessage, StreamEvent, StreamMsg, client_message, server_message,
};

impl From<SendMsg> for ClientMessage {
    fn from(msg: SendMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Send(msg)),
        }
    }
}

impl From<StreamMsg> for ClientMessage {
    fn from(msg: StreamMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Stream(msg)),
        }
    }
}

impl From<SendResponse> for ServerMessage {
    fn from(r: SendResponse) -> Self {
        Self {
            msg: Some(server_message::Msg::Response(r)),
        }
    }
}

impl From<StreamEvent> for ServerMessage {
    fn from(e: StreamEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::Stream(e)),
        }
    }
}

impl From<AgentEventMsg> for ServerMessage {
    fn from(e: AgentEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::AgentEvent(e)),
        }
    }
}

impl From<McpEventMsg> for ServerMessage {
    fn from(e: McpEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::McpEvent(e)),
        }
    }
}

impl From<ConversationHistory> for ServerMessage {
    fn from(h: ConversationHistory) -> Self {
        Self {
            msg: Some(server_message::Msg::ConversationHistory(h)),
        }
    }
}

#[cfg(feature = "prost_guest")]
mod message {
    use crate::{SendResponse, ServerMessage, server_message};

    impl ServerMessage {
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

    impl TryFrom<ServerMessage> for SendResponse {
        type Error = anyhow::Error;
        fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
            match msg.msg {
                Some(server_message::Msg::Response(r)) => Ok(r),
                _ => Err(msg.error_or_unexpected()),
            }
        }
    }

    impl TryFrom<ServerMessage> for crate::stream_event::Event {
        type Error = anyhow::Error;
        fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
            match msg.msg {
                Some(server_message::Msg::Stream(e)) => {
                    e.event.ok_or_else(|| anyhow::anyhow!("empty stream event"))
                }
                _ => Err(msg.error_or_unexpected()),
            }
        }
    }
}
