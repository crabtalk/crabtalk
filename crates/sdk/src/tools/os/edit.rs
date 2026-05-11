//! Edit tool schema and handler.

use super::{MAX_FILE_SIZE, OsHook};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Replace an exact string in a file. Fails if the string is not found or appears more than once.
#[derive(Deserialize, JsonSchema)]
pub struct Edit {
    /// Path to the file to edit.
    pub path: String,
    /// Exact string to find and replace. Must appear exactly once in the file.
    pub old_string: String,
    /// Replacement string.
    pub new_string: String,
}

impl OsHook {
    pub(super) async fn handle_edit(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Edit =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;

        if input.old_string.is_empty() {
            return Err("old_string must not be empty".to_owned());
        }
        if input.old_string == input.new_string {
            return Err("old_string and new_string are identical".to_owned());
        }

        let cwd = self.effective_cwd(call.conversation_id);

        let path = if std::path::Path::new(&input.path).is_absolute() {
            std::path::PathBuf::from(&input.path)
        } else {
            cwd.join(&input.path)
        };

        if !self.was_read(call.conversation_id, &path) {
            return Err(format!(
                "you must read {} before editing it",
                path.display()
            ));
        }

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
    }
}
