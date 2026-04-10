//! Memory domain types.
//!
//! [`MemoryEntry`] is a named memory blob with YAML frontmatter metadata.

use anyhow::{Result, bail};

/// A single memory entry.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// Human-readable name. Primary key for the repo (slugified for fs).
    pub name: String,
    /// One-line description used for relevance scoring.
    pub description: String,
    /// Entry body (markdown content).
    pub content: String,
}

impl MemoryEntry {
    /// Parse an entry from its frontmatter-based file content.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.replace("\r\n", "\n");
        let raw = raw.trim();
        if !raw.starts_with("---") {
            bail!("missing frontmatter opening ---");
        }

        let after_open = &raw[3..];
        let Some(close_pos) = after_open.find("\n---") else {
            bail!("missing frontmatter closing ---");
        };

        let frontmatter = &after_open[..close_pos];
        let content = after_open[close_pos + 4..].trim().to_owned();

        let mut name = None;
        let mut description = None;

        for line in frontmatter.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("name:") {
                name = Some(val.trim().to_owned());
            } else if let Some(val) = line.strip_prefix("description:") {
                description = Some(val.trim().to_owned());
            }
        }

        let Some(name) = name else {
            bail!("missing 'name' in frontmatter");
        };
        let description = description.unwrap_or_default();

        Ok(Self {
            name,
            description,
            content,
        })
    }

    /// Serialize to the frontmatter file format.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&format!("name: {}\n", self.name));
        out.push_str(&format!("description: {}\n", self.description));
        out.push_str("---\n\n");
        out.push_str(&self.content);
        out.push('\n');
        out
    }

    /// Text for BM25 scoring — description + content concatenated.
    pub fn search_text(&self) -> String {
        format!("{} {}", self.description, self.content)
    }
}

/// Convert a name to a filesystem-safe slug.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut prev_dash = true;

    for ch in name.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                slug.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }

    if slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        slug.push_str("entry");
    }

    slug
}
