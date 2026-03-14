//! Walrus hub — unified registry for all download operations.
//!
//! The "hub" encompasses walrus packages, proxied huggingface models,
//! embeddings pre-download, and future skill downloads. Each operation
//! gets a unique ID, tracked status, and broadcasts events to subscribers.

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::broadcast;
use tokio::time::Instant;
use wcore::protocol::message::server::{DownloadEvent, DownloadInfo, DownloadKind};

pub mod embeddings;
pub mod manifest;
pub mod package;

// ── Registry ──────────────────────────────────────────────────────

/// Download status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    /// Download in progress.
    Downloading,
    /// Download completed successfully.
    Completed,
    /// Download failed.
    Failed,
}

impl std::fmt::Display for DownloadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Downloading => write!(f, "downloading"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// A tracked download operation.
pub struct Download {
    pub id: u64,
    pub kind: DownloadKind,
    pub label: String,
    pub status: DownloadStatus,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
    pub created_at: Instant,
}

/// In-memory download registry with broadcast event channel.
pub struct DownloadRegistry {
    downloads: BTreeMap<u64, Download>,
    next_id: AtomicU64,
    broadcast: broadcast::Sender<DownloadEvent>,
}

impl Default for DownloadRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
        let (broadcast, _) = broadcast::channel(64);
        Self {
            downloads: BTreeMap::new(),
            next_id: AtomicU64::new(1),
            broadcast,
        }
    }

    /// Subscribe to download lifecycle events.
    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.broadcast.subscribe()
    }

    /// Register a new download, returning its ID.
    pub fn start(&mut self, kind: DownloadKind, label: String) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let download = Download {
            id,
            kind: kind.clone(),
            label: label.clone(),
            status: DownloadStatus::Downloading,
            bytes_downloaded: 0,
            total_bytes: 0,
            error: None,
            created_at: Instant::now(),
        };
        self.downloads.insert(id, download);
        let _ = self
            .broadcast
            .send(DownloadEvent::Created { id, kind, label });
        id
    }

    /// Report byte-level progress for a download.
    pub fn progress(&mut self, id: u64, bytes: u64, total_bytes: u64) {
        if let Some(dl) = self.downloads.get_mut(&id) {
            dl.bytes_downloaded += bytes;
            dl.total_bytes = total_bytes;
        }
        let _ = self.broadcast.send(DownloadEvent::Progress {
            id,
            bytes,
            total_bytes,
        });
    }

    /// Report a human-readable progress step.
    pub fn step(&mut self, id: u64, message: String) {
        let _ = self.broadcast.send(DownloadEvent::Step { id, message });
    }

    /// Mark a download as completed.
    pub fn complete(&mut self, id: u64) {
        if let Some(dl) = self.downloads.get_mut(&id) {
            dl.status = DownloadStatus::Completed;
        }
        let _ = self.broadcast.send(DownloadEvent::Completed { id });
    }

    /// Mark a download as failed.
    pub fn fail(&mut self, id: u64, error: String) {
        if let Some(dl) = self.downloads.get_mut(&id) {
            dl.status = DownloadStatus::Failed;
            dl.error = Some(error.clone());
        }
        let _ = self.broadcast.send(DownloadEvent::Failed { id, error });
    }

    /// List all downloads, most recent first.
    pub fn list(&self) -> Vec<DownloadInfo> {
        self.downloads
            .values()
            .rev()
            .map(|dl| DownloadInfo {
                id: dl.id,
                kind: dl.kind.clone(),
                label: dl.label.clone(),
                status: dl.status.to_string(),
                bytes_downloaded: dl.bytes_downloaded,
                total_bytes: dl.total_bytes,
                error: dl.error.clone(),
                alive_secs: dl.created_at.elapsed().as_secs(),
            })
            .collect()
    }
}
