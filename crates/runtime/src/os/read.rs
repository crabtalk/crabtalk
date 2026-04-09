//! read tool — paginated file reading with line numbers.

use crate::{Env, host::Host};
use schemars::JsonSchema;
use serde::Deserialize;
use std::fmt::Write;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

/// Default maximum number of lines to return per read.
const DEFAULT_LIMIT: usize = 2000;

/// Maximum file size in bytes before refusing to read (50 MB).
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

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

pub fn tools() -> Vec<Tool> {
    vec![Read::as_tool()]
}

impl<H: Host> Env<H> {
    pub async fn dispatch_read(
        &self,
        args: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        let input: Read =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

        let conversation_cwd = if let Some(id) = conversation_id {
            self.host.conversation_cwd(id)
        } else {
            None
        };
        let cwd = conversation_cwd.as_deref().unwrap_or(&self.cwd);

        let path = if std::path::Path::new(&input.path).is_absolute() {
            std::path::PathBuf::from(&input.path)
        } else {
            cwd.join(&input.path)
        };

        // Size guard — refuse to read files that could OOM the process.
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
        let start = offset - 1; // convert 1-based to 0-based

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
    }
}
