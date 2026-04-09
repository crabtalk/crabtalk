//! Memory entry — frontmatter-based file format for individual memories.

use crate::memory::ENTRIES_PREFIX;
use anyhow::{Result, bail};
use wcore::Storage;

/// A single memory entry.
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub content: String,
    /// Storage key for this entry (e.g. `memory/entries/<slug>.md`).
    pub key: String,
}

impl MemoryEntry {
    /// Create a new entry with a computed storage key under the shared
    /// entries prefix.
    pub fn new(name: String, description: String, content: String) -> Self {
        let slug = slugify(&name);
        let key = format!("{ENTRIES_PREFIX}{slug}.md");
        Self {
            name,
            description,
            content,
            key,
        }
    }

    /// Parse an entry from its file content and its storage key.
    pub fn parse(key: String, raw: &str) -> Result<Self> {
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
            key,
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

    /// Write this entry to storage.
    pub fn save(&self, storage: &dyn Storage) -> Result<()> {
        storage.put(&self.key, self.serialize().as_bytes())
    }

    /// Delete this entry from storage.
    pub fn delete(&self, storage: &dyn Storage) -> Result<()> {
        storage.delete(&self.key)
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
