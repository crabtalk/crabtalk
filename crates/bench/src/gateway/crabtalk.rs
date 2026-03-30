//! Crabtalk gateway — sends tasks via protobuf over TCP.

use crate::{
    gateway::{Gateway, TaskResult, timed},
    task::Task,
};
use gateway::client::DaemonClient;
use wcore::protocol::message::{SendMsg, server_message};

pub struct CrabtalkGateway {
    port: u16,
}

impl CrabtalkGateway {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

impl Gateway for CrabtalkGateway {
    fn run_task(&self, rt: &tokio::runtime::Runtime, task: &Task) -> TaskResult {
        let port = self.port;
        let prompt = task.prompt.to_string();
        rt.block_on(async move {
            timed(|| async {
                let client = DaemonClient::tcp(port);
                let msg = SendMsg {
                    agent: "crab".into(),
                    content: prompt,
                    sender: Some("bench".into()),
                    new_chat: true,
                    ..Default::default()
                };
                let mut rx = client.send(msg.into()).await;

                let mut response = String::new();
                while let Some(server_msg) = rx.recv().await {
                    match server_msg.msg {
                        Some(server_message::Msg::Response(r)) => {
                            response = r.content;
                            break;
                        }
                        Some(server_message::Msg::Stream(event)) => {
                            if let Some(wcore::protocol::message::stream_event::Event::Chunk(
                                chunk,
                            )) = event.event
                            {
                                response.push_str(&chunk.content);
                            }
                        }
                        Some(server_message::Msg::Error(e)) => {
                            return Err(format!("daemon error: {}", e.message));
                        }
                        _ => {}
                    }
                }
                Ok(response)
            })
            .await
        })
    }
}
