//! Pluggable KV storage backend.
//!
//! Runtime subsystems (memory, skills, sessions, agents, event bus,
//! cron) and core conversation persistence go through the [`Storage`]
//! trait instead of reaching for `std::fs` directly. The trait lives
//! in `crabtalk-core` because both `crabtalk-runtime` and
//! `wcore::runtime::Conversation` are consumers; the daemon provides
//! the concrete filesystem implementation
//! (`crabtalk_daemon::storage::FsStorage`).
//!
//! # Shape
//!
//! Four methods: [`get`](Storage::get), [`put`](Storage::put),
//! [`delete`](Storage::delete), [`list`](Storage::list). Keys are flat
//! `/`-separated strings owned by the caller (e.g. `memory/entries/<id>.md`,
//! `agents/<ulid>/prompt.md`). Values are bytes — callers handle their own
//! UTF-8, TOML, or JSON decoding.
//!
//! The trait is synchronous. Runtime subsystems already block on
//! small-file I/O from async contexts; forcing an async-trait boundary
//! here would propagate `.await` through many call sites for no benefit.
//! An async variant can land as a separate concern if a backend ever
//! needs it.

use anyhow::Result;

pub mod mem;

pub use mem::MemStorage;

/// Key/value byte store used by runtime subsystems for persistence.
pub trait Storage: Send + Sync {
    /// Fetch the bytes stored at `key`, or `Ok(None)` if the key does not
    /// exist. Errors are reserved for backend failures (I/O, permissions).
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Write `value` to `key`, replacing any existing entry. Implementors
    /// should make this write atomic w.r.t. reads from other threads —
    /// concurrent readers must see either the old or new value, never a
    /// partial write.
    fn put(&self, key: &str, value: &[u8]) -> Result<()>;

    /// Remove the entry at `key`. Returns `Ok(())` whether or not the key
    /// existed — delete is idempotent.
    fn delete(&self, key: &str) -> Result<()>;

    /// List keys starting with `prefix`, in sorted order. The prefix is
    /// matched literally (no glob semantics). An empty prefix lists every
    /// key in the store.
    fn list(&self, prefix: &str) -> Result<Vec<String>>;
}
