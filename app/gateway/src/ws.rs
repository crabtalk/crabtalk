//! WebSocket server -- axum upgrade handler and message loop.

use crate::auth::{AuthContext, Authenticator};
use crate::session::SessionScope;
use crate::state::AppState;
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    response::IntoResponse,
    routing::get,
};
use compact_str::CompactString;
use futures_util::{SinkExt, StreamExt};
use llm::Message;
use protocol::{ClientMessage, ServerMessage};
use runtime::Hook;
use std::collections::BTreeMap;
use tokio::sync::mpsc;

/// Build the axum router with the `/ws` endpoint.
pub fn router<H: Hook + 'static, A: Authenticator + 'static>(state: AppState<H, A>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler::<H, A>))
        .with_state(state)
}

/// WebSocket upgrade handler.
async fn ws_handler<H: Hook + 'static, A: Authenticator + 'static>(
    State(state): State<AppState<H, A>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle an established WebSocket connection.
async fn handle_socket<H: Hook + 'static, A: Authenticator + 'static>(
    socket: WebSocket,
    state: AppState<H, A>,
) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();

    // Sender task: forward ServerMessages to the WebSocket.
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("failed to serialize server message: {e}");
                    continue;
                }
            };
            if sender.send(WsMessage::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    let mut auth_context: Option<AuthContext> = None;
    let mut session_histories: BTreeMap<CompactString, Vec<Message>> = BTreeMap::new();

    // Receiver loop: process incoming ClientMessages.
    while let Some(Ok(ws_msg)) = receiver.next().await {
        let text = match ws_msg {
            WsMessage::Text(t) => t,
            WsMessage::Close(_) => break,
            _ => continue,
        };

        let client_msg: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    code: 400,
                    message: format!("invalid message: {e}"),
                });
                continue;
            }
        };

        match client_msg {
            ClientMessage::Authenticate { token } => {
                match state.authenticator.authenticate(&token).await {
                    Ok(ctx) => {
                        let session = state.sessions.create(SessionScope::Main, ctx.trust_level);
                        auth_context = Some(ctx);
                        let _ = tx.send(ServerMessage::Authenticated {
                            session_id: session.id,
                        });
                    }
                    Err(_) => {
                        let _ = tx.send(ServerMessage::Error {
                            code: 401,
                            message: "authentication failed".to_string(),
                        });
                    }
                }
            }

            ClientMessage::Send { agent, content } => {
                if auth_context.is_none() {
                    let _ = tx.send(ServerMessage::Error {
                        code: 401,
                        message: "not authenticated".to_string(),
                    });
                    continue;
                }

                let history = session_histories.entry(agent.clone()).or_default();
                match state
                    .runtime
                    .send_stateless(&agent, history, &content)
                    .await
                {
                    Ok(response) => {
                        let _ = tx.send(ServerMessage::Response {
                            agent,
                            content: response,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(ServerMessage::Error {
                            code: 500,
                            message: format!("agent error: {e}"),
                        });
                    }
                }
            }

            ClientMessage::Stream { agent, content } => {
                if auth_context.is_none() {
                    let _ = tx.send(ServerMessage::Error {
                        code: 401,
                        message: "not authenticated".to_string(),
                    });
                    continue;
                }

                // For now, use non-streaming send and wrap in stream messages.
                // Full streaming (using provider.stream()) is a follow-up.
                let _ = tx.send(ServerMessage::StreamStart {
                    agent: agent.clone(),
                });

                let history = session_histories.entry(agent.clone()).or_default();
                match state
                    .runtime
                    .send_stateless(&agent, history, &content)
                    .await
                {
                    Ok(response) => {
                        let _ = tx.send(ServerMessage::StreamChunk { content: response });
                        let _ = tx.send(ServerMessage::StreamEnd { agent });
                    }
                    Err(e) => {
                        let _ = tx.send(ServerMessage::Error {
                            code: 500,
                            message: format!("stream error: {e}"),
                        });
                    }
                }
            }

            ClientMessage::ClearSession { agent } => {
                session_histories.remove(&agent);
                let _ = tx.send(ServerMessage::SessionCleared { agent });
            }

            ClientMessage::Ping => {
                let _ = tx.send(ServerMessage::Pong);
            }
        }
    }

    // Clean up.
    drop(tx);
    let _ = send_task.await;
}
