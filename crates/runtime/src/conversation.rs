//! Conversation history persistence — reading JSONL files from `~/.crabtalk/sessions/`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};
use wcore::model::Message;

/// Conversation metadata (first line of a JSONL conversation file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub agent: String,
    pub created_by: String,
    pub created_at: String,
}

/// List all persisted conversations. Returns `(filename_stem, meta)` pairs.
pub fn list_conversations(conversations_dir: &Path) -> Result<Vec<(String, ConversationMeta)>> {
    let mut results = Vec::new();

    let entries = match fs::read_dir(conversations_dir) {
        Ok(e) => e,
        Err(_) => return Ok(results),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();

        if let Ok(meta) = read_meta(&path) {
            results.push((stem, meta));
        }
    }

    results.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
    Ok(results)
}

/// Load a full conversation from disk: metadata + all messages.
pub fn load_conversation(path: &Path) -> Result<(ConversationMeta, Vec<Message>)> {
    let file = fs::File::open(path)
        .with_context(|| format!("open conversation file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let meta_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty conversation file"))?
        .context("read meta line")?;
    let meta: ConversationMeta =
        serde_json::from_str(&meta_line).context("parse conversation meta")?;

    let mut messages = Vec::new();
    for line in lines {
        let line = line.context("read message line")?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Message = serde_json::from_str(&line).context("parse message")?;
        messages.push(msg);
    }

    Ok((meta, messages))
}

/// Read just the metadata (first line) from a conversation file.
fn read_meta(path: &Path) -> Result<ConversationMeta> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(line.trim())?)
}
