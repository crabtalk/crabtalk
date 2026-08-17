//! DDL, applied on open. Every statement is `IF NOT EXISTS`, so opening
//! an existing database is a no-op and there is no migration table to
//! keep in step.

/// Write transactions take the reserved lock up front. SQLite's default
/// deferred transaction promotes on first write, which turns a concurrent
/// writer into `SQLITE_BUSY` mid-transaction instead of at the start.
pub(super) const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

pub(super) const DDL: &[&str] = &[
    // One row per conversation thread. `archive` names the memory entry
    // holding the compacted prefix; null until the first compact.
    "CREATE TABLE IF NOT EXISTS sessions (
        handle        TEXT    PRIMARY KEY,
        agent         TEXT    NOT NULL,
        created_by    TEXT    NOT NULL,
        title         TEXT    NOT NULL DEFAULT '',
        created_at    TEXT    NOT NULL,
        updated_at    TEXT    NOT NULL DEFAULT '',
        message_count INTEGER NOT NULL DEFAULT 0,
        summary       TEXT,
        archive       TEXT
    )",
    // Serves `find_latest_session`, which is on the path of every resume.
    "CREATE INDEX IF NOT EXISTS sessions_agent_creator
        ON sessions(agent, created_by, created_at DESC)",
    // The conversation's HistoryEntry stream, one entry per row.
    "CREATE TABLE IF NOT EXISTS session_messages (
        session_handle TEXT    NOT NULL REFERENCES sessions(handle) ON DELETE CASCADE,
        idx            INTEGER NOT NULL,
        entry_json     TEXT    NOT NULL,
        PRIMARY KEY (session_handle, idx)
    )",
    // EventLine trace stream. `kind` is indexed for rollups like
    // `WHERE kind = 'done'` to total token usage for a session.
    "CREATE TABLE IF NOT EXISTS session_events (
        session_handle TEXT    NOT NULL REFERENCES sessions(handle) ON DELETE CASCADE,
        idx            INTEGER NOT NULL,
        ts             TEXT    NOT NULL,
        kind           TEXT    NOT NULL,
        payload_json   TEXT    NOT NULL,
        PRIMARY KEY (session_handle, idx)
    )",
    "CREATE INDEX IF NOT EXISTS session_events_kind
        ON session_events(session_handle, kind)",
    // Free-text search over messages. FTS5 ships `bm25()`, so the ranking
    // is the database's rather than a resident index the process rebuilds
    // at boot. `role` rides along unindexed so weighting is one CASE in the
    // ORDER BY instead of a second pass in Rust.
    "CREATE VIRTUAL TABLE IF NOT EXISTS session_search USING fts5(
        body,
        session_handle UNINDEXED,
        idx            UNINDEXED,
        role           UNINDEXED
    )",
    // Title and summary rank a session as a whole, so they are their own
    // documents rather than extra columns on every message.
    "CREATE VIRTUAL TABLE IF NOT EXISTS session_meta_search USING fts5(
        title,
        summary,
        session_handle UNINDEXED
    )",
    // The config is stored whole rather than as a column per field.
    // Nothing queries inside an `AgentConfig`, and a column layout would
    // need a migration every time the struct gains a field — which it
    // does (RFC 0193 replaced `mcps: Vec<String>` with full configs).
    // `name` is a column because lookup by name is a trait method.
    "CREATE TABLE IF NOT EXISTS agents (
        id            TEXT PRIMARY KEY,
        name          TEXT NOT NULL UNIQUE,
        config_json   TEXT NOT NULL
    )",
    // Install config, one row. Kept in the database so a tenant is one
    // file — the point of a database per tenant.
    "CREATE TABLE IF NOT EXISTS config (
        id   INTEGER PRIMARY KEY CHECK (id = 1),
        toml TEXT NOT NULL
    )",
];
