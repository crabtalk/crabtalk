//! Filesystem-backed persistence.
//!
//! [`FsStorage`] implements [`Storage`](wcore::storage::Storage)
//! with TOML configs, markdown prompts, and JSON session files.

pub use self::fs::FsStorage;
pub use backfill::{backfill_local_agent_ids, migrate_local_agent_prompts};
pub use loader::{DEFAULT_CONFIG, scaffold_config_dir};
use std::{
    fs as stdfs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

mod backfill;
mod fs;
mod loader;

/// Atomic write: same-directory tmp file + rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut tmp_os = path.to_path_buf().into_os_string();
    tmp_os.push(format!(".tmp.{}.{}", std::process::id(), nanos));
    let tmp_path = PathBuf::from(tmp_os);
    stdfs::write(&tmp_path, data)?;
    if let Err(e) = stdfs::rename(&tmp_path, path) {
        let _ = stdfs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}
