//! Storage abstraction for memory persistence.

use anyhow::Result;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::RwLock,
};

/// Abstraction over filesystem operations used by the memory system.
pub trait Storage: Send + Sync {
    fn read(&self, path: &Path) -> Result<String>;
    fn write(&self, path: &Path, content: &str) -> Result<()>;
    fn delete(&self, path: &Path) -> Result<()>;
    fn list(&self, dir: &Path) -> Result<Vec<PathBuf>>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
}

/// Production storage backed by `std::fs`.
pub struct FsStorage;

impl Storage for FsStorage {
    fn read(&self, path: &Path) -> Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }

    fn write(&self, path: &Path, content: &str) -> Result<()> {
        Ok(std::fs::write(path, content)?)
    }

    fn delete(&self, path: &Path) -> Result<()> {
        Ok(std::fs::remove_file(path)?)
    }

    fn list(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            paths.push(entry?.path());
        }
        Ok(paths)
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        Ok(std::fs::create_dir_all(path)?)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        Ok(std::fs::rename(from, to)?)
    }
}

/// In-memory storage for tests. No disk I/O.
pub struct MemStorage {
    files: RwLock<HashMap<PathBuf, String>>,
}

impl Default for MemStorage {
    fn default() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
        }
    }
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemStorage {
    fn read(&self, path: &Path) -> Result<String> {
        self.files
            .read()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("file not found: {}", path.display()))
    }

    fn write(&self, path: &Path, content: &str) -> Result<()> {
        self.files
            .write()
            .unwrap()
            .insert(path.to_path_buf(), content.to_owned());
        Ok(())
    }

    fn delete(&self, path: &Path) -> Result<()> {
        self.files
            .write()
            .unwrap()
            .remove(path)
            .ok_or_else(|| anyhow::anyhow!("file not found: {}", path.display()))?;
        Ok(())
    }

    fn list(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let files = self.files.read().unwrap();
        Ok(files
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect())
    }

    fn create_dir_all(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.read().unwrap().contains_key(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let mut files = self.files.write().unwrap();
        let content = files
            .remove(from)
            .ok_or_else(|| anyhow::anyhow!("file not found: {}", from.display()))?;
        files.insert(to.to_path_buf(), content);
        Ok(())
    }
}
