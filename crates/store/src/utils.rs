//! Helpers that belong to no one type.

use anyhow::Result;

/// Reject names that won't survive serialization as a TOML table key.
pub fn validate_table_name(kind: &str, name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("{kind}: name must not be empty");
    }
    if name
        .chars()
        .any(|c| matches!(c, '.' | '[' | ']' | '"') || c.is_control())
    {
        anyhow::bail!(
            "{kind}: name '{name}' must not contain '.', '[', ']', '\"', or control chars"
        );
    }
    Ok(())
}
