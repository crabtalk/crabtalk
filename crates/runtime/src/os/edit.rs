//! edit tool — exact string replacement with conflict detection.

use crate::{Env, host::Host};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

use crate::os::read::MAX_FILE_SIZE;

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

pub fn tools() -> Vec<Tool> {
    vec![Edit::as_tool()]
}

impl<H: Host> Env<H> {
    pub async fn dispatch_edit(
        &self,
        args: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        let input: Edit =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

        if input.old_string.is_empty() {
            return Err("old_string must not be empty".to_owned());
        }
        if input.old_string == input.new_string {
            return Err("old_string and new_string are identical".to_owned());
        }

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
