//! Conversations — the thing that runs here, so this is the `ps`.

use crate::{conn, table};
use anyhow::Result;
use proto::api::Client as _;

/// Live conversations, or every one the store has when `all`.
pub async fn run(all: bool) -> Result<()> {
    let mut transport = conn::connect().await?;
    let (empty_agent, empty_sender) = (String::new(), String::new());

    let rows: Vec<Vec<String>> = if all {
        transport
            .list_conversations(empty_agent, empty_sender)
            .await?
            .into_iter()
            .map(|c| {
                vec![
                    c.file_path,
                    c.agent_name,
                    c.sender,
                    c.message_count.to_string(),
                    table::duration(c.alive_secs),
                    table::truncate(&c.title, 40),
                ]
            })
            .collect()
    } else {
        transport
            .list_active_conversations(empty_agent, empty_sender)
            .await?
            .into_iter()
            .map(|c| {
                vec![
                    c.session_handle,
                    c.agent_name,
                    c.sender,
                    c.message_count.to_string(),
                    table::duration(c.alive_secs),
                    table::truncate(&c.title, 40),
                ]
            })
            .collect()
    };

    table::print(
        &["HANDLE", "AGENT", "SENDER", "MESSAGES", "AGE", "TITLE"],
        &rows,
    );
    Ok(())
}
