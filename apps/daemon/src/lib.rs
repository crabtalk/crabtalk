//! Crabtalk daemon — foreground startup.

use anyhow::Result;
use std::{path::PathBuf, sync::Arc};
use storage::FsStorage;

pub mod foreground;

fn ensure_config() -> Result<()> {
    storage::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
    Ok(())
}

/// The backend this daemon runs on. Picking one is the binary's job —
/// `crabtalk` is generic over `Storage` and is handed the choice, so an
/// embedder can hand it a different one without forking the library.
pub(crate) fn build_storage(config_dir: &std::path::Path) -> Arc<FsStorage> {
    let dirs = wcore::resolve_dirs(config_dir);
    let skill_roots: Vec<PathBuf> = dirs
        .skill_dirs
        .iter()
        .filter(|dir| dir.exists())
        .cloned()
        .collect();
    Arc::new(FsStorage::new(
        config_dir.to_path_buf(),
        config_dir.join("sessions"),
        skill_roots,
    ))
}
