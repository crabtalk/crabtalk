//! Shared text embedder for Crabtalk: all-MiniLM-L6-v2 (384-dim) on Candle (pure
//! Rust, CPU — no ONNX runtime). Cloud and desktop both embed through this one
//! crate, so a vector produced on either side is comparable *by construction*,
//! not merely by "using the same model".
//!
//! Weights load from a directory the consumer populated with `config.json`,
//! `tokenizer.json`, and `model.safetensors`, each verified against a pinned
//! sha256 on load. This crate bundles no weights and carries no HTTP client —
//! fetching them is a deployment concern, not the embedder's.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
mod model;

/// Identifier stored next to each vector; bump it on a model swap so consumers
/// re-embed. Held equal to cloud's original value so existing vectors stay valid.
pub const MODEL_ID: &str = "minilm-l6-v2-st";
/// Output dimensions — any `VECTOR(384)` column or stored BLOB must agree.
pub const DIM: u32 = 384;

/// sha256 (hex) of the three model files, verified on load.
pub(crate) const CONFIG_SHA256: &str =
    "953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41";
pub(crate) const TOKENIZER_SHA256: &str =
    "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037";
pub(crate) const MODEL_SHA256: &str =
    "53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db";

/// A lazily-loaded embedder over a model directory. Cheap to construct; the model
/// loads (and verifies) on first embed and is cached for the embedder's lifetime.
pub struct Embedder {
    dir: PathBuf,
    inner: Mutex<Option<Arc<model::Model>>>,
}

impl Embedder {
    /// Embed against the model in `dir` — a directory holding `config.json`,
    /// `tokenizer.json`, and `model.safetensors`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            inner: Mutex::new(None),
        }
    }

    /// Mean-pooled embeddings, one row per input. Rows are **unnormalized** —
    /// call [`l2_normalize`] before storing or comparing so cosine is a plain dot
    /// product on both sides.
    pub fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.model()?.embed(texts)
    }

    fn model(&self) -> Result<Arc<model::Model>, EmbedError> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(model) = guard.as_ref() {
            return Ok(model.clone());
        }
        let loaded = Arc::new(model::load(&self.dir)?);
        *guard = Some(loaded.clone());
        Ok(loaded)
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
