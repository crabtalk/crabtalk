//! Session search, answered by FTS5.
//!
//! SQLite ships `bm25()`, so ranking is the database's job. Role weighting
//! is a multiplier in the `ORDER BY` — `bm25()` returns negative scores
//! where more-negative is a better match, so a weight above 1 promotes.

use crate::backend::SqliteStorage;
use anyhow::Result;
use std::str::FromStr;
use store::{
    AgentId, SessionHandle,
    session::{MAX_HITS_PER_QUERY, SearchOptions, SessionHit},
};

/// One row of the hit query: handle, message index, score, then the
/// session's own metadata, then the agent's name — left-joined, so it is
/// empty for a session whose agent has since been deleted.
type HitRow = (
    String,
    i64,
    f64,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
);

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
                        s.title, s.agent, a.name, s.created_by, s.created_at, s.updated_at
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
                 LEFT JOIN agents a ON a.id = s.agent
                 WHERE h.rn = 1
                   AND (?2 IS NULL OR s.agent = ?2)
                   AND (?3 IS NULL OR s.created_by = ?3)
                 ORDER BY h.score
                 LIMIT ?4",
        )
        .bind(&matcher)
        .bind(opts.agent_filter.map(|id| id.to_string()))
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
        for (handle, idx, score, title, agent, agent_name, sender, created_at, updated_at) in rows {
            let boost = titled.contains(&handle) as u8 as f64 * TITLE_BOOST
                + summarised.contains(&handle) as u8 as f64 * SUMMARY_BOOST;
            hits.push(SessionHit {
                session_handle: SessionHandle::new(handle),
                msg_idx: idx as u32,
                // `bm25()` is negative-is-better; the wire wants the opposite.
                score: -score + boost,
                title,
                agent: AgentId::from_str(&agent)?,
                agent_name: agent_name.unwrap_or_default(),
                sender,
                created_at,
                updated_at,
                // Content is KV's; the caller fills this.
                window: Vec::new(),
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
}

/// Quote a user query as an FTS5 string literal so punctuation is terms
/// rather than query syntax.
pub(super) fn fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    terms.join(" ")
}
