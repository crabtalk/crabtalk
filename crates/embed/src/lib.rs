//! Shared text embedder for Crabtalk: multilingual-e5-small (384-dim) on Candle.
//! Cloud and desktop embed through this one crate, so a vector produced on either
//! side is comparable *by construction*, not merely by "using the same model".
//!
//! No bundled weights and no HTTP client — fetching them is a deployment concern.
//! The consumer supplies a directory holding the three [`FILES`].
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
mod model;

/// Stored next to each vector, so a model swap is detectable and stale vectors
/// get rebuilt.
pub const MODEL_ID: &str = "multilingual-e5-small";
/// Output dimensions — any `VECTOR(384)` column or stored BLOB must agree.
pub const DIM: u32 = 384;

/// The three files a model directory must hold, each pinned by sha256 and
/// verified on load. Canonical source: the `intfloat/multilingual-e5-small` repo.
pub const FILES: [(&str, &str); 3] = [
    ("config.json", CONFIG_SHA256),
    ("tokenizer.json", TOKENIZER_SHA256),
    ("model.safetensors", MODEL_SHA256),
];

pub(crate) const CONFIG_SHA256: &str =
    "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959";
pub(crate) const TOKENIZER_SHA256: &str =
    "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39";
pub(crate) const MODEL_SHA256: &str =
    "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477";

/// Which side of an asymmetric retrieval a text is. e5 prefixes the two
/// differently, and a pair must agree for cosine between them to mean anything.
#[derive(Debug, Clone, Copy)]
pub enum EmbedRole {
    Query,
    Document,
}

impl EmbedRole {
    fn prefix(self) -> &'static str {
        match self {
            EmbedRole::Query => "query: ",
            EmbedRole::Document => "passage: ",
        }
    }
}

/// Cheap to construct; the model loads and verifies on first embed, then caches.
pub struct Embedder {
    dir: PathBuf,
    inner: Mutex<Option<Arc<model::Model>>>,
}

impl Embedder {
    /// Embed against the model in `dir` — a directory holding the three [`FILES`].
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            inner: Mutex::new(None),
        }
    }

    /// Mean-pooled embeddings, one row per input, prefixed for `role`. Rows are
    /// **unnormalized** — call [`l2_normalize`] before storing or comparing.
    pub fn embed(&self, texts: Vec<String>, role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        let prefixed = texts
            .into_iter()
            .map(|t| format!("{}{t}", role.prefix()))
            .collect();
        self.model()?.embed(prefixed)
    }

    /// Recovers from poisoning — a panicked cache holder is no reason to bring
    /// the process down.
    fn cache(&self) -> MutexGuard<'_, Option<Arc<model::Model>>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn model(&self) -> Result<Arc<model::Model>, EmbedError> {
        let mut guard = self.cache();
        if let Some(model) = guard.as_ref() {
            return Ok(model.clone());
        }
        let loaded = Arc::new(model::load(&self.dir)?);
        *guard = Some(loaded.clone());
        Ok(loaded)
    }

    /// Whether the model is currently resident.
    pub fn is_loaded(&self) -> bool {
        self.cache().is_some()
    }

    /// Release the weights; the next embed reloads and re-verifies from `dir`.
    /// Lets a consumer hold the memory only between bursts.
    pub fn unload(&self) {
        *self.cache() = None;
    }
}

/// L2-normalize in place; normalized vectors make cosine a plain dot product, so
/// a pgvector `<=>` and a stored-BLOB dot product agree on distance.
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Lowercase-hex sha256, used to verify each model file against its pin.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Errors from loading the model or embedding text.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("load model: {0}")]
    Load(String),
    #[error("inference: {0}")]
    Inference(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("model file {file}: sha256 mismatch (expected {expected}, got {got})")]
    Hash {
        file: String,
        expected: String,
        got: String,
    },
}
