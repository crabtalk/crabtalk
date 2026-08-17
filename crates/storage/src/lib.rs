//! Persistence backends for Crabtalk.
//!
//! [`Storage`](wcore::storage::Storage) is declared in core; this crate
//! implements it. A backend is chosen at compile time through
//! `runtime::Config`'s `Storage` associated type, not by a feature — the
//! `sqlite` feature exists only so a consumer that wants the filesystem
//! doesn't build a SQL driver it will never call.
//!
//! [`FsStorage`] is the daemon's backend: TOML configs, markdown prompts,
//! and JSON session files under `~/.crabtalk/`.

pub mod backend;
