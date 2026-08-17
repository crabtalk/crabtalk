//! Session search, answered by FTS5.
//!
//! SQLite ships `bm25()`, so ranking is the database's job. Role weighting
//! is a multiplier in the `ORDER BY` — `bm25()` returns negative scores
//! where more-negative is a better match, so a weight above 1 promotes.

use crate::{
    HistoryEntry, SessionHandle,
    backend::sqlite::SqliteStorage,
    session::{
        MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHit,
        WindowItem,
    },
};
use anyhow::Result;
use crabllm_core::{Role, anthropic::ContentBlock};

/// One row of the hit query: handle, message index, score, then the
/// session's own metadata.
type HitRow = (String, i64, f64, String, String, String, String, String);

/// How much a title or summary match adds to a session's best message hit.
const TITLE_BOOST: f64 = 2.0;
const SUMMARY_BOOST: f64 = 3.0;

impl SqliteStorage {
    pub(super) async fn search_sessions(
        &self,
        query: &str,
        opts: &SearchOptions,
    ) -> Result<Vec<SessionHit>> {
        let limit = opts.limit.clamp(1, MAX_HITS_PER_QUERY) as i64;
        // FTS5 would treat bare punctuation as syntax; quote the whole query
        // so a user's `foo(bar)` is terms rather than a parse error.
        let matcher = fts_query(query);
        if matcher.is_empty() {
            return Ok(Vec::new());
        }

        // `bm25()` needs the FTS5 match cursor, so it only exists in the
        // query that carries the MATCH. The window function then ranks the
        // materialised score, one nesting level out.
        let rows: Vec<HitRow> = sqlx::query_as(
            "SELECT h.session_handle, h.idx, h.score,
                        s.title, s.agent, s.created_by, s.created_at, s.updated_at
                 FROM (
                     SELECT session_handle, idx, score,
                            ROW_NUMBER() OVER (
                                PARTITION BY session_handle ORDER BY score
                            ) AS rn
                     FROM (
                         SELECT session_handle, idx,
                                bm25(session_search) * CASE role
                                    WHEN 'user'           THEN 1.5
                                    WHEN 'assistant_tool' THEN 1.3
                                    ELSE 1.0
                                END AS score
                         FROM session_search
                         WHERE session_search MATCH ?1
                     )
                 ) h
                 JOIN sessions s ON s.handle = h.session_handle
                 WHERE h.rn = 1
                   AND (?2 IS NULL OR s.agent = ?2)
                   AND (?3 IS NULL OR s.created_by = ?3)
                 ORDER BY h.score
                 LIMIT ?4",
        )
        .bind(&matcher)
        .bind(opts.agent_filter.as_deref())
        .bind(opts.sender_filter.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        // Title and summary rank the session as a whole, so they lift
        // whichever message hit that session already produced. Column-scoped
        // MATCH keeps the two boosts distinct.
        let titled = self.meta_matches("title", &matcher).await;
        let summarised = self.meta_matches("summary", &matcher).await;

        let mut hits = Vec::with_capacity(rows.len());
        for (handle, idx, score, title, agent, sender, created_at, updated_at) in rows {
            let boost = titled.contains(&handle) as u8 as f64 * TITLE_BOOST
                + summarised.contains(&handle) as u8 as f64 * SUMMARY_BOOST;
            let handle = SessionHandle::new(handle);
            let window = self.window(&handle, idx, opts).await?;
            hits.push(SessionHit {
                session_handle: handle,
                msg_idx: idx as u32,
                // `bm25()` is negative-is-better; the wire wants the opposite.
                score: -score + boost,
                title,
                agent,
                sender,
                created_at,
                updated_at,
                window,
            });
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(hits)
    }

    /// Session handles whose `column` matches the query.
    async fn meta_matches(&self, column: &str, matcher: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT session_handle FROM session_meta_search
             WHERE session_meta_search MATCH ?1",
        )
        .bind(format!("{{{column}}} : ({matcher})"))
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }

    /// The messages surrounding a hit, bounded by [`MAX_WINDOW_ITEMS`].
    async fn window(
        &self,
        handle: &SessionHandle,
        idx: i64,
        opts: &SearchOptions,
    ) -> Result<Vec<WindowItem>> {
        let before = opts.context_before.min(MAX_WINDOW_ITEMS) as i64;
        let after = opts.context_after.min(MAX_WINDOW_ITEMS) as i64;
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT idx, entry_json FROM session_messages
             WHERE session_handle = ?1 AND idx BETWEEN ?2 AND ?3
             ORDER BY idx LIMIT ?4",
        )
        .bind(handle.as_str())
        .bind(idx - before)
        .bind(idx + after)
        .bind(MAX_WINDOW_ITEMS as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(i, json)| {
                let entry: HistoryEntry = serde_json::from_str(&json).ok()?;
                let (snippet, truncated) = snippet(&entry);
                Some(WindowItem {
                    role: entry.role().clone(),
                    msg_idx: i as u32,
                    snippet,
                    truncated,
                    tool_name: tool_name(&entry),
                })
            })
            .collect())
    }
}

/// Text fed to FTS5. User and assistant messages contribute their content;
/// tool-call assistants contribute only the function names (arguments may
/// carry secrets); tool-result and system messages contribute nothing, to
/// keep credentials out of free-text search.
pub(super) fn indexable(entry: &HistoryEntry) -> Option<(String, &'static str)> {
    if entry.auto_injected {
        return None;
    }
    let role = entry.role().clone();
    if !matches!(role, Role::User | Role::Assistant) || has_tool_result(entry) {
        return None;
    }
    let text = entry.text();
    if !text.is_empty() {
        let tag = if matches!(role, Role::User) {
            "user"
        } else {
            "assistant"
        };
        return Some((text.to_owned(), tag));
    }
    let names: Vec<_> = entry
        .tool_calls()
        .iter()
        .map(|tc| tc.function.name.clone())
        .collect();
    if names.is_empty() {
        return None;
    }
    let tag = if matches!(role, Role::User) {
        "user"
    } else {
        "assistant_tool"
    };
    Some((names.join(" "), tag))
}

/// Quote a user query as an FTS5 string literal so punctuation is terms
/// rather than query syntax.
fn fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    terms.join(" ")
}

fn has_tool_result(entry: &HistoryEntry) -> bool {
    entry
        .message
        .blocks()
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
}

fn snippet(entry: &HistoryEntry) -> (String, bool) {
    let raw = entry.text().to_owned();
    if raw.len() <= MAX_SNIPPET_BYTES {
        return (raw, false);
    }
    let mut end = MAX_SNIPPET_BYTES;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    (raw[..end].to_owned(), true)
}

fn tool_name(entry: &HistoryEntry) -> Option<String> {
    for block in entry.message.blocks() {
        match block {
            ContentBlock::ToolResult { name: Some(n), .. } if !n.is_empty() => {
                return Some(n.clone());
            }
            ContentBlock::ToolUse { name, .. } => return Some(name.clone()),
            _ => {}
        }
    }
    None
}
