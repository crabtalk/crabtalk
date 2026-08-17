//! An append-only key-value store, in one file.
//!
//! Better than a directory of files — which is the bar it was written
//! to clear, not to beat a database. A directory costs an inode per key,
//! gives no ordered iteration, and offers no atomicity; this packs
//! records into one file, keeps a resident key index so lookups are one
//! seek and prefix scans are an ordered walk, and survives a crash by
//! discarding a torn tail.
//!
//! Two pieces: [`mod@format`] is the CRMEM layout on disk, [`CrabDb`] is the
//! store over it.
//!
//! ```no_run
//! # fn main() -> anyhow::Result<()> {
//! let db = crabtalk_crabdb::CrabDb::open("store.crmem")?;
//! db.put(0, b"agent/one", b"{}")?;
//! assert_eq!(db.get(0, b"agent/one")?.as_deref(), Some(&b"{}"[..]));
//! db.checkpoint()?;
//! # Ok(()) }
//! ```

pub use db::{CrabDb, Options};
pub use format::Key;

pub mod format;

mod db;
