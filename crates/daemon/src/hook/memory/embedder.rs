//! Candle-based text embedder using all-MiniLM-L6-v2 for 384-dim sentence
//! embeddings. Downloads model files from HF Hub on first use, caches under
//! `~/.openwalrus/.cache/huggingface/`.

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use hf_hub::{Repo, RepoType, api::sync::ApiBuilder};
use std::path::Path;
use tokenizers::Tokenizer;

const MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Sentence embedder backed by candle's BERT implementation.
pub(crate) struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
}

impl Embedder {
    /// Load the all-MiniLM-L6-v2 model, downloading from HF Hub if needed.
    /// `cache_dir` controls where model files are stored on disk.
    pub(crate) fn load(cache_dir: &Path) -> Result<Self> {
        let api = ApiBuilder::new()
            .with_cache_dir(cache_dir.to_path_buf())
            .with_progress(true)
            .build()
            .context("failed to build HF Hub API")?;
        let repo = api.repo(Repo::new(MODEL_ID.into(), RepoType::Model));

        let config_path = repo
            .get("config.json")
            .context("failed to fetch config.json")?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("failed to fetch tokenizer.json")?;
        let weights_path = repo
            .get("model.safetensors")
            .context("failed to fetch model.safetensors")?;

        let config: Config = serde_json::from_str(
            &std::fs::read_to_string(&config_path).context("failed to read config.json")?,
        )
        .context("failed to parse config.json")?;

        let device = Device::Cpu;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                .context("failed to load model weights")?
        };
        let model = BertModel::load(vb, &config).context("failed to load BertModel")?;

        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!("{e}"))?;

        Ok(Self { model, tokenizer })
    }

    /// Generate a normalized 384-dim embedding vector for the given text.
    pub(crate) fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let device = &self.model.device;
        let ids = Tensor::new(encoding.get_ids(), device)?.unsqueeze(0)?;
        let type_ids = ids.zeros_like()?;
        let mask = Tensor::new(encoding.get_attention_mask(), device)?.unsqueeze(0)?;

        // Forward pass → [1, seq_len, hidden_size]
        let token_embeddings = self.model.forward(&ids, &type_ids, Some(&mask))?;

        // Mean-pool over non-padding tokens.
        let mask_f = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let sum_mask = mask_f.sum(1)?;
        let pooled = token_embeddings.broadcast_mul(&mask_f)?.sum(1)?;
        let pooled = pooled.broadcast_div(&sum_mask)?;

        // L2-normalize.
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm)?;

        // Extract as Vec<f32>.
        let embedding: Vec<f32> = normalized.squeeze(0)?.to_vec1()?;
        Ok(embedding)
    }
}
