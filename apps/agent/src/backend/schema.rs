//! DDL, applied on open. Every statement is `IF NOT EXISTS`, so opening
//! an existing database is a no-op and there is no migration table to
//! keep in step.
//!
//! Two tables. Content and every secondary index live in `kv`, because
//! an index is just more keys; `text_index` exists because ranking prose
//! against a query is the one thing a keyspace cannot do. There is no
//! table per entity — what a session or an agent *is* never reaches SQL.

pub(super) const DDL: &[&str] = &[
    // The KV primitive. `col` is a hard partition, so a prefix scan
    // cannot cross kinds, and the primary key makes a scan an index seek.
    "CREATE TABLE IF NOT EXISTS kv (
        col   INTEGER NOT NULL,
        key   BLOB    NOT NULL,
        value BLOB    NOT NULL,
        PRIMARY KEY (col, key)
    )",
    // The text primitive. FTS5 ships `bm25()`, so ranking is the
    // database's rather than a resident index the process rebuilds at
    // boot. `ix` separates documents drawn from one keyspace — a
    // session's messages and its title are both `Column::Session` — and
    // `weight` rides along so the caller's notion of what matters more
    // is applied in the ORDER BY rather than in a second pass.
    "CREATE VIRTUAL TABLE IF NOT EXISTS text_index USING fts5(
        body,
        ix     UNINDEXED,
        key    UNINDEXED,
        weight UNINDEXED
    )",
];
