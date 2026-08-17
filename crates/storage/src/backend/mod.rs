//! Persistence backends for Crabtalk.

mod fs;
mod sqlite;

#[cfg(feature = "fs")]
pub use fs::FsStorage;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStorage;
