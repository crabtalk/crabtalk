//! `impl Harnesses for Store`.

use crate::{
    interface::Harnesses,
    kv::{Column, KVStorage},
    sql::SqlIndex,
    store::Store,
};
use anyhow::Result;

impl<K: KVStorage, Q: SqlIndex> Harnesses for Store<K, Q> {
    async fn harness_image(&self, digest: &str) -> Result<Option<Vec<u8>>> {
        let key = self.tenant.key(&["image", digest]);
        self.kv.get(Column::Harness, &key).await
    }

    async fn put_harness_image(&self, name: &str, bytes: &[u8]) -> Result<String> {
        let digest = digest(bytes);
        // The image is immutable under its digest, so re-putting the same
        // bytes is a no-op and two agents declaring one harness share the
        // entry. Only the name→digest pointer moves.
        self.kv
            .put(
                Column::Harness,
                &self.tenant.key(&["image", &digest]),
                bytes,
            )
            .await?;
        self.kv
            .put(
                Column::Harness,
                &self.tenant.key(&["name", name]),
                digest.as_bytes(),
            )
            .await?;
        Ok(digest)
    }

    async fn resolve_harness(&self, name: &str) -> Result<Option<String>> {
        let key = self.tenant.key(&["name", name]);
        let Some(bytes) = self.kv.get(Column::Harness, &key).await? else {
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
