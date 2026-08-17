//! DDL, applied on open. Every statement is `IF NOT EXISTS`, so opening
//! an existing database is a no-op and there is no migration table to
//! keep in step.

pub(super) const DDL: &[&str] = &[
    // One row per session thread. `archive` names the memory entry
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
    // Agent identity only. The config itself is KV content; what lives
    // here is what a lookup needs to find it — the id it is keyed by and
    // the name a person addresses it with.
    "CREATE TABLE IF NOT EXISTS agents (
        id   TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE
    )",
    // The KV primitive. Content addressed by a key the caller already
    // holds; `col` is the hard partition, so a prefix scan cannot cross
    // kinds. A dedicated KV engine (parity-db) is another impl of the
    // same trait — this one exists so a local install stays one file.
    "CREATE TABLE IF NOT EXISTS kv (
        col   INTEGER NOT NULL,
        key   BLOB    NOT NULL,
        value BLOB    NOT NULL,
        PRIMARY KEY (col, key)
    )",
    // Memory entries are content in KV; this indexes their text so
    // recall is a query rather than a resident BM25 index rebuilt at
    // boot. `name` is the KV key the hit points back at.
    "CREATE VIRTUAL TABLE IF NOT EXISTS memory_search USING fts5(
        body,
        name UNINDEXED
    )",
    "CREATE TABLE IF NOT EXISTS memory_index (
        name TEXT PRIMARY KEY
    )",
    // Skill identity, without bodies. A listing reads this table; the
    // markdown is a KV read for the skill actually invoked.
    "CREATE TABLE IF NOT EXISTS skills_index (
        name        TEXT PRIMARY KEY,
        description TEXT NOT NULL DEFAULT ''
    )",
];
