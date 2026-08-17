//! The SKILL.md format — YAML frontmatter, then the body.

use crate::Skill;
use serde::{Deserialize, Deserializer};
use std::{collections::BTreeMap, str::FromStr};

/// Accept both `"a, b, c"` (string) and `["a", "b", "c"]` (sequence) for tool lists.
fn string_or_vec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }
    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::Vec(v) => Ok(v),
        StringOrVec::String(s) => Ok(s
            .split([',', ' '])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()),
    }
}

/// YAML frontmatter deserialization target for SKILL.md files.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default, rename = "allowed-tools", deserialize_with = "string_or_vec")]
    allowed_tools: Vec<String>,
}

impl FromStr for Skill {
    type Err = anyhow::Error;

    fn from_str(content: &str) -> anyhow::Result<Self> {
        let (frontmatter, body) = split(content)?;
        let fm: Frontmatter = serde_yml::from_str(frontmatter)?;

        Ok(Self {
            name: fm.name,
            description: fm.description,
            license: fm.license,
            compatibility: fm.compatibility,
            metadata: fm.metadata,
            allowed_tools: fm.allowed_tools,
            body: body.to_owned(),
        })
    }
}

/// Split YAML frontmatter from the body. Frontmatter is delimited by `---`.
///
/// Handles CRLF line endings and trailing whitespace on delimiter lines.
pub(crate) fn split(content: &str) -> anyhow::Result<(&str, &str)> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        anyhow::bail!("missing YAML frontmatter delimiter (---)");
    }

    // Skip opening delimiter and its trailing newline.
    let after_first = content[3..].trim_start_matches(['\n', '\r']);

    // Scan line-by-line for the closing `---` delimiter.
    let mut pos = 0;
    for line in after_first.lines() {
        if line.trim() == "---" {
            let frontmatter = &after_first[..pos].trim_end();
            let body_start = pos + line.len();
            // Skip the newline after `---` if present.
            let body = after_first[body_start..].trim_start_matches(['\n', '\r']);
            return Ok((frontmatter, body));
        }
        pos += line.len() + 1; // +1 for the newline consumed by lines()
    }

    anyhow::bail!("missing closing YAML frontmatter delimiter (---)")
}
