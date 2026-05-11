//! Read tool schema and handler.

use super::{MAX_FILE_SIZE, OsHook};
use schemars::JsonSchema;
use serde::Deserialize;
use std::fmt::Write;
use wcore::ToolDispatch;

/// Default maximum number of lines to return per read.
const DEFAULT_LIMIT: usize = 2000;

/// Read a file with line numbers. Supports offset/limit for pagination.
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

impl OsHook {
    pub(super) async fn handle_read(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Read =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        let cwd = self.effective_cwd(call.conversation_id);

        let path = if std::path::Path::new(&input.path).is_absolute() {
            std::path::PathBuf::from(&input.path)
        } else {
            cwd.join(&input.path)
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

        if let Some(id) = call.conversation_id {
            self.record_read(id, path);
        }

        Ok(buf)
    }
}
