//! OS tool handler factories — bash, read, edit.

use runtime::ConversationCwds;
use schemars::JsonSchema;
use serde::Deserialize;
use std::{collections::BTreeMap, fmt::Write, path::PathBuf, sync::Arc};
use wcore::{
    ToolDispatch, ToolEntry,
    agent::{AsTool, ToolDescription},
};

/// Default maximum number of lines to return per read.
const DEFAULT_LIMIT: usize = 2000;

/// Maximum file size in bytes before refusing to read (50 MB).
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

// ── Schemas ──────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema)]
pub struct Bash {
    /// Shell command to run (e.g. `"ls -la"`, `"cat foo.txt | grep bar"`).
    pub command: String,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl ToolDescription for Bash {
    const DESCRIPTION: &'static str = "Run a shell command.";
}

#[derive(Deserialize, JsonSchema)]
pub struct Read {
    /// Absolute or relative file path to read.
    pub path: String,
    /// Line number to start reading from (1-based). Defaults to 1.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of lines to read. Defaults to 2000.
    #[serde(default)]
    pub limit: Option<usize>,
}

impl ToolDescription for Read {
    const DESCRIPTION: &'static str =
        "Read a file with line numbers. Supports offset/limit for pagination.";
}

#[derive(Deserialize, JsonSchema)]
pub struct Edit {
    /// Path to the file to edit.
    pub path: String,
    /// Exact string to find and replace. Must appear exactly once in the file.
    pub old_string: String,
    /// Replacement string.
    pub new_string: String,
}

impl ToolDescription for Edit {
    const DESCRIPTION: &'static str = "Replace an exact string in a file. Fails if the string is not found or appears more than once.";
}

// ── Handlers ─────────────────────────────────────────────────────

/// Build an `<environment>` XML block with OS info.
fn environment_block() -> String {
    let mut buf = String::from("\n\n<environment>\n");
    let _ = writeln!(buf, "os: {}", std::env::consts::OS);
    buf.push_str("</environment>");
    buf
}

pub fn bash(cwd: PathBuf, cwds: ConversationCwds) -> ToolEntry {
    ToolEntry {
        schema: Bash::as_tool(),
        system_prompt: Some(environment_block()),
        before_run: None,
        handler: Arc::new(move |call: ToolDispatch| {
            let cwds = cwds.clone();
            let cwd = cwd.clone();
            Box::pin(async move {
                if call.sender.contains(':') {
                    return Err("bash is only available in the command line interface".to_owned());
                }
                let input: Bash = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                let conversation_cwd = if let Some(id) = call.conversation_id {
                    cwds.lock().await.get(&id).cloned()
                } else {
                    None
                };
                let effective_cwd = conversation_cwd.as_deref().unwrap_or(&cwd);

                let mut cmd = tokio::process::Command::new("bash");
                cmd.args(["-c", &input.command])
                    .envs(&input.env)
                    .current_dir(effective_cwd)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                let child = cmd.spawn().map_err(|e| {
                    serde_json::json!({
                        "stdout": "",
                        "stderr": format!("bash failed: {e}"),
                        "exit_code": -1
                    })
                    .to_string()
                })?;

                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    child.wait_with_output(),
                )
                .await
                {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let exit_code = output.status.code().unwrap_or(-1);
                        Ok(serde_json::json!({
                            "stdout": stdout.as_ref(),
                            "stderr": stderr.as_ref(),
                            "exit_code": exit_code
                        })
                        .to_string())
                    }
                    Ok(Err(e)) => Err(serde_json::json!({
                        "stdout": "",
                        "stderr": format!("bash failed: {e}"),
                        "exit_code": -1
                    })
                    .to_string()),
                    Err(_) => Err(serde_json::json!({
                        "stdout": "",
                        "stderr": "bash timed out after 30 seconds",
                        "exit_code": -1
                    })
                    .to_string()),
                }
            })
        }),
    }
}

pub fn read(cwd: PathBuf, cwds: ConversationCwds) -> ToolEntry {
    ToolEntry {
        schema: Read::as_tool(),
        system_prompt: None,
        before_run: None,
        handler: Arc::new(move |call: ToolDispatch| {
            let cwds = cwds.clone();
            let cwd = cwd.clone();
            Box::pin(async move {
                let input: Read = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                let conversation_cwd = if let Some(id) = call.conversation_id {
                    cwds.lock().await.get(&id).cloned()
                } else {
                    None
                };
                let effective_cwd = conversation_cwd.as_deref().unwrap_or(&cwd);

                let path = if std::path::Path::new(&input.path).is_absolute() {
                    std::path::PathBuf::from(&input.path)
                } else {
                    effective_cwd.join(&input.path)
                };

                match std::fs::metadata(&path) {
                    Ok(m) if m.len() > MAX_FILE_SIZE => {
                        return Err(format!(
                            "file is too large ({} bytes, max {})",
                            m.len(),
                            MAX_FILE_SIZE
                        ));
                    }
                    Err(e) => return Err(format!("error reading {}: {e}", path.display())),
                    _ => {}
                }

                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("error reading {}: {e}", path.display()))?;

                let total = content.lines().count();
                let offset = input.offset.unwrap_or(1).max(1);
                let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
                let start = offset - 1;

                if start >= total {
                    return Ok(format!(
                        "--- {total} total lines (offset {offset} is past end of file) ---"
                    ));
                }

                let mut buf = String::new();
                let mut shown = 0;
                for (line_num, line) in content.lines().skip(start).take(limit).enumerate() {
                    let _ = writeln!(buf, "{}\t{line}", start + line_num + 1);
                    shown += 1;
                }

                let end = start + shown;
                if start > 0 || end < total {
                    let _ = write!(
                        buf,
                        "\n--- {total} total lines (showing lines {}-{end}) ---",
                        start + 1,
                    );
                } else {
                    let _ = write!(buf, "\n--- {total} total lines ---");
                }

                Ok(buf)
            })
        }),
    }
}

pub fn edit(cwd: PathBuf, cwds: ConversationCwds) -> ToolEntry {
    ToolEntry {
        schema: Edit::as_tool(),
        system_prompt: None,
        before_run: None,
        handler: Arc::new(move |call: ToolDispatch| {
            let cwds = cwds.clone();
            let cwd = cwd.clone();
            Box::pin(async move {
                let input: Edit = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;

                if input.old_string.is_empty() {
                    return Err("old_string must not be empty".to_owned());
                }
                if input.old_string == input.new_string {
                    return Err("old_string and new_string are identical".to_owned());
                }

                let conversation_cwd = if let Some(id) = call.conversation_id {
                    cwds.lock().await.get(&id).cloned()
                } else {
                    None
                };
                let effective_cwd = conversation_cwd.as_deref().unwrap_or(&cwd);

                let path = if std::path::Path::new(&input.path).is_absolute() {
                    std::path::PathBuf::from(&input.path)
                } else {
                    effective_cwd.join(&input.path)
                };

                match std::fs::metadata(&path) {
                    Ok(m) if m.len() > MAX_FILE_SIZE => {
                        return Err(format!(
                            "file is too large ({} bytes, max {})",
                            m.len(),
                            MAX_FILE_SIZE
                        ));
                    }
                    Err(e) => return Err(format!("error reading {}: {e}", path.display())),
                    _ => {}
                }

                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("error reading {}: {e}", path.display()))?;

                let count = content.matches(&input.old_string).count();
                if count == 0 {
                    return Err("old_string not found".to_owned());
                }
                if count > 1 {
                    return Err(format!(
                        "old_string is not unique, found {count} occurrences"
                    ));
                }

                let new_content = content.replacen(&input.old_string, &input.new_string, 1);
                std::fs::write(&path, &new_content)
                    .map_err(|e| format!("error writing {}: {e}", path.display()))?;

                Ok("ok".to_owned())
            })
        }),
    }
}
