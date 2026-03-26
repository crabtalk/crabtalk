//! Session history persistence — reading JSONL files from `~/.crabtalk/sessions/`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};
use wcore::model::Message;

/// Session metadata (first line of a JSONL session file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub agent: String,
    pub created_by: String,
    pub created_at: String,
}

/// List all persisted sessions. Returns `(filename_stem, meta)` pairs.
pub fn list_sessions(sessions_dir: &Path) -> Result<Vec<(String, SessionMeta)>> {
    let mut results = Vec::new();

    let entries = match fs::read_dir(sessions_dir) {
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

/// Load a full session from disk: metadata + all messages.
pub fn load_session(path: &Path) -> Result<(SessionMeta, Vec<Message>)> {
    let file =
        fs::File::open(path).with_context(|| format!("open session file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let meta_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty session file"))?
        .context("read meta line")?;
    let meta: SessionMeta = serde_json::from_str(&meta_line).context("parse session meta")?;

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

/// Read just the metadata (first line) from a session file.
fn read_meta(path: &Path) -> Result<SessionMeta> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(line.trim())?)
}
