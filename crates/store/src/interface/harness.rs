//! Harness images, addressed by digest.

use crate::kv::{Column, KVStorage};
use anyhow::Result;
use std::future::Future;

/// Harness images, addressed by digest.
///
/// The digest is the identity: an image already loaded under one is the
/// same sandbox, so residency needs no invalidation and two agents
/// declaring the same harness share one instantiation. `resolve_harness`
/// is the only mutable part — the name that currently points at a digest.
///
/// Shaped for extraction: berm will own this interface once it is split
/// out, and these three methods are what it needs.
pub trait Harnesses: Send + Sync + 'static {
    fn harness_image(&self, digest: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store an image and return the digest it is keyed by.
    fn put_harness_image(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<String>> + Send;

    fn resolve_harness(&self, name: &str) -> impl Future<Output = Result<Option<String>>> + Send;
}

impl<T: KVStorage> Harnesses for T {
    async fn harness_image(&self, digest: &str) -> Result<Option<Vec<u8>>> {
        self.get(Column::Harness, &self.key(&["image", digest]))
            .await
    }

    async fn put_harness_image(&self, name: &str, bytes: &[u8]) -> Result<String> {
        let digest = digest(bytes);
        // The image is immutable under its digest, so re-putting the
        // same bytes is a no-op and two agents declaring one harness
        // share the entry. Only the name→digest pointer moves.
        self.put(Column::Harness, &self.key(&["image", &digest]), bytes)
            .await?;
        self.put(
            Column::Harness,
            &self.key(&["name", name]),
            digest.as_bytes(),
        )
        .await?;
        Ok(digest)
    }

    async fn resolve_harness(&self, name: &str) -> Result<Option<String>> {
        let Some(bytes) = self
            .get(Column::Harness, &self.key(&["name", name]))
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(String::from_utf8(bytes)?))
    }
}

/// Content address for a harness image.
fn digest(bytes: &[u8]) -> String {
    // FNV-1a: berm keys images by digest only to tell "same image" from
    // "different image", and this store never sees an adversary that
    // picks the bytes — the daemon does not download code.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}
