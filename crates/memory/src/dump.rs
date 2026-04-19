//! Dump / load: canonical serialization of the db as a markdown tree.
//!
//! Layout:
//! ```text
//! brain/
//!   SUMMARY.md                ← auto-generated mdbook ToC (ignored on load)
//!   notes/{name}.md           ← EntryKind::Note
//!   archives/{name}.md        ← EntryKind::Archive
//!   topics/{name}.md          ← EntryKind::Topic
//! ```
//!
//! Entry file format: an HTML metadata block at the top, then the
//! entry's content as pure markdown:
//!
//! ```markdown
//! <div id="meta">
//! <dl>
//!   <dt>Created</dt>
//!   <dd><time datetime="2026-04-14T10:23:45Z">2026-04-14T10:23:45Z</time></dd>
//!   <dt>Aliases</dt>
//!   <dd>
//!     <ul>
//!       <li>ship</li>
//!       <li>release</li>
//!     </ul>
//!   </dd>
//! </dl>
//! </div>
//!
//! prod rollout steps ...
//! ```
//!
//! Uses `<dl>` / `<dt>` / `<dd>` — the semantic HTML for key-value
//! metadata. Browsers render it as a labeled info card; mdbook doesn't
//! pull the labels into its heading tree, so they can't collide with
//! any headings the content itself uses. The `<time datetime="...">`
//! attribute round-trips the exact unix timestamp. The `Aliases` row
//! is omitted when the entry has none. A file that does not start
//! with `<div id="meta">` is treated as pure content with no metadata.

use crate::{
    entry::{Entry, EntryKind},
    error::{Error, Result},
};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use std::{collections::HashMap, fs, path::Path};

/// All kinds that the dump tree knows about, paired with their
/// subdirectory name and SUMMARY section title.
pub(crate) const KIND_SECTIONS: &[(EntryKind, &str, &str)] = &[
    (EntryKind::Note, "notes", "Notes"),
    (EntryKind::Archive, "archives", "Archives"),
    (EntryKind::Topic, "topics", "Topics"),
];

const META_OPEN: &str = "<div id=\"meta\">";
const META_CLOSE: &str = "</div>";

/// Minimal `book.toml` so the dumped tree is `mdbook serve`-ready.
/// `src = "."` points mdbook at the dump root where `SUMMARY.md` lives,
/// instead of the default `src/` subdirectory.
pub(crate) const BOOK_TOML: &str = "[book]\ntitle = \"Memory\"\nsrc = \".\"\n\n[output.html]\n";

/// Reject names that aren't safe as filesystem basenames. POSIX-friendly;
/// does not check Windows-reserved basenames (CON, NUL, etc.) — the dump
/// tree is not intended to roundtrip across Windows.
pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name.contains("..")
        || name.chars().any(|c| c.is_control())
    {
        return Err(Error::InvalidName(name.to_owned()));
    }
    Ok(())
}

pub(crate) fn serialize_entry(e: &Entry) -> String {
    let iso = format_ts(e.created_at);
    let mut out = String::new();
    out.push_str(META_OPEN);
    out.push('\n');
    out.push_str("<dl>\n");
    out.push_str("  <dt>Created</dt>\n");
    out.push_str(&format!(
        "  <dd><time datetime=\"{iso}\">{iso}</time></dd>\n"
    ));
    if !e.aliases.is_empty() {
        out.push_str("  <dt>Aliases</dt>\n");
        out.push_str("  <dd>\n    <ul>\n");
        for a in &e.aliases {
            out.push_str(&format!("      <li>{}</li>\n", html_escape(a)));
        }
        out.push_str("    </ul>\n  </dd>\n");
    }
    out.push_str("</dl>\n");
    out.push_str(META_CLOSE);
    out.push_str("\n\n");
    let body = e.content.trim_start_matches('\n');
    out.push_str(body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub(crate) struct Parsed {
    pub(crate) created_at: Option<u64>,
    pub(crate) aliases: Vec<String>,
    pub(crate) content: String,
}

/// Parse a dumped entry markdown file.
pub(crate) fn parse_entry(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let Some(rest) = text.strip_prefix(META_OPEN) else {
        return Parsed {
            created_at: None,
            aliases: Vec::new(),
            content: text.trim_end().to_owned(),
        };
    };
    let Some(end) = rest.find(META_CLOSE) else {
        return Parsed {
            created_at: None,
            aliases: Vec::new(),
            content: text.trim_end().to_owned(),
        };
    };
    let meta = &rest[..end];
    let body = rest[end + META_CLOSE.len()..].trim_start().trim_end();

    Parsed {
        created_at: extract_created(meta),
        aliases: extract_aliases(meta),
        content: body.to_owned(),
    }
}

fn extract_created(meta: &str) -> Option<u64> {
    let attr_start = meta.find("datetime=\"")? + "datetime=\"".len();
    let attr_len = meta[attr_start..].find('"')?;
    let iso = &meta[attr_start..attr_start + attr_len];
    let dt = DateTime::parse_from_rfc3339(iso).ok()?;
    let secs = dt.timestamp();
    if secs < 0 { None } else { Some(secs as u64) }
}

fn extract_aliases(meta: &str) -> Vec<String> {
    let Some(header) = meta.find("<dt>Aliases</dt>") else {
        return Vec::new();
    };
    let after = &meta[header..];
    // Bound the scan to this row — aliases end at the next <dt>.
    let bound = after[1..]
        .find("<dt>")
        .map(|i| i + 1)
        .unwrap_or(after.len());
    let row = &after[..bound];

    let mut out = Vec::new();
    let mut rest = row;
    while let Some(li_start) = rest.find("<li>") {
        let open_end = li_start + "<li>".len();
        let Some(li_end) = rest[open_end..].find("</li>") else {
            break;
        };
        let inner = &rest[open_end..open_end + li_end];
        out.push(html_unescape(inner.trim()));
        rest = &rest[open_end + li_end + "</li>".len()..];
    }
    out
}

fn format_ts(secs: u64) -> String {
    let dt: DateTime<Utc> = Utc
        .timestamp_opt(secs as i64, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap());
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

pub(crate) fn build_summary(by_kind: &HashMap<EntryKind, Vec<&Entry>>) -> String {
    let mut out = String::from("# Summary\n\n");
    for (kind, dir, title) in KIND_SECTIONS {
        let Some(entries) = by_kind.get(kind) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        let mut sorted: Vec<&&Entry> = entries.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        out.push_str("# ");
        out.push_str(title);
        out.push_str("\n\n");
        for e in sorted {
            out.push_str(&format!("- [{name}]({dir}/{name}.md)\n", name = e.name));
        }
        out.push('\n');
    }
    out
}

pub(crate) struct Loaded {
    pub(crate) kind: EntryKind,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) created_at: Option<u64>,
}

/// Walk the tree and collect every markdown file as a [`Loaded`].
pub(crate) fn read_tree(dir: &Path) -> Result<Vec<Loaded>> {
    let mut out = Vec::new();
    for (kind, sub, _) in KIND_SECTIONS {
        let path = dir.join(sub);
        if !path.is_dir() {
            continue;
        }
        let mut names: Vec<_> = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
        names.sort_by_key(|e| e.file_name());
        for ent in names {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(name) = p.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let raw = fs::read_to_string(&p)?;
            let parsed = parse_entry(&raw);
            out.push(Loaded {
                kind: *kind,
                name,
                content: parsed.content,
                aliases: parsed.aliases,
                created_at: parsed.created_at,
            });
        }
    }
    Ok(out)
}
