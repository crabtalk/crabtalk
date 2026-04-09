//! `std::fs`-backed [`Storage`] implementation.

use anyhow::{Context, Result};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use wcore::Storage;

/// Filesystem key/value store rooted at a single directory. Keys are
/// flat `/`-separated strings that map 1:1 to relative paths under
/// `root`. Writes are atomic via same-directory tmp file + rename.
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    /// Create a new `FsStorage` rooted at the given directory. The root
    /// is not created eagerly — `put` creates parent directories as
    /// needed.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    /// Convert an absolute path under `root` into a storage key (forward
    /// slashes, no leading separator). Returns `None` if the path is not
    /// under `root` (defensive — shouldn't happen in practice).
    fn path_to_key(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut key = String::with_capacity(rel.as_os_str().len());
        for (i, component) in rel.components().enumerate() {
            if i > 0 {
                key.push('/');
            }
            key.push_str(&component.as_os_str().to_string_lossy());
        }
        Some(key)
    }
}

impl Storage for FsStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.key_to_path(key);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        let final_path = self.key_to_path(key);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
        // Same-directory tmp file so the rename stays on one filesystem
        // and therefore stays atomic. Pid + nanos suffix keeps the tmp
        // name unique across concurrent writers and across crashes.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut tmp_os = final_path.clone().into_os_string();
        tmp_os.push(format!(".tmp.{}.{}", std::process::id(), nanos));
        let tmp_path = PathBuf::from(tmp_os);
        fs::write(&tmp_path, value).with_context(|| format!("write {}", tmp_path.display()))?;
        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            // Best-effort cleanup; the failing rename is the real error.
            let _ = fs::remove_file(&tmp_path);
            return Err(e).with_context(|| format!("rename to {}", final_path.display()));
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = self.key_to_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("delete {}", path.display())),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        // Start the walk at the deepest directory implied by the prefix
        // to avoid touching unrelated parts of the tree. Everything up to
        // the last `/` is a directory path; anything after is a basename
        // filter applied on top of the prefix match.
        let dir_prefix = match prefix.rfind('/') {
            Some(idx) => &prefix[..=idx],
            None => "",
        };
        let walk_root = self.key_to_path(dir_prefix);
        if !walk_root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        walk_dir(&walk_root, self, prefix, &mut out)?;
        out.sort();
        Ok(out)
    }
}

/// Recursively walk `dir`, collecting files whose storage key starts
/// with `prefix`. Symlinks to directories are intentionally not
/// followed — we only persist files this backend wrote.
fn walk_dir(dir: &Path, storage: &FsStorage, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read dir entry in {}", dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("file type of {}", entry.path().display()))?;
        if file_type.is_dir() {
            walk_dir(&entry.path(), storage, prefix, out)?;
        } else if file_type.is_file() {
            if let Some(key) = storage.path_to_key(&entry.path())
                && key.starts_with(prefix)
            {
                out.push(key);
            }
        }
    }
    Ok(())
}
