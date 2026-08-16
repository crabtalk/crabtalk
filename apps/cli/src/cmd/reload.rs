//! Hot-reload the running daemon's configuration.

use anyhow::Result;
use wcore::protocol::api::Client;
use wcore::protocol::message::{ClientMessage, client_message};

pub async fn run(tcp: bool) -> Result<()> {
    let (mut transport, _) = super::connect(tcp).await?;
    let msg = ClientMessage {
        msg: Some(client_message::Msg::Reload(Default::default())),
    };
    transport.request(msg).await?;
    println!("daemon reloaded");
    Ok(())
}
