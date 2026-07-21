//! The BERT model behind the embedder: load the three files from a directory
//! into a Candle `BertModel` + tokenizer, then mean-pool (attention-masked) the
//! token states into one vector per input.
use crate::EmbedError;
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use std::path::Path;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer};

pub(crate) struct Model {
    bert: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl Model {
    pub(crate) fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.embed_inner(texts)
            .map_err(|e| EmbedError::Inference(e.to_string()))
    }

    fn embed_inner(&self, texts: Vec<String>) -> candle_core::Result<Vec<Vec<f32>>> {
        let encodings = self
            .tokenizer
            .encode_batch(texts, true)
            .map_err(|e| candle_core::Error::Msg(e.to_string()))?;
        let ids = encodings
            .iter()
            .map(|e| Tensor::new(e.get_ids(), &self.device))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let mask = encodings
            .iter()
            .map(|e| Tensor::new(e.get_attention_mask(), &self.device))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let ids = Tensor::stack(&ids, 0)?;
        let mask = Tensor::stack(&mask, 0)?;
        let type_ids = ids.zeros_like()?;
        let hidden = self.bert.forward(&ids, &type_ids, Some(&mask))?;
        // Mean-pool over tokens, weighted by the attention mask so padding
        // doesn't dilute the vector.
        let mask = mask.to_dtype(DTYPE)?.unsqueeze(2)?;
        let summed = hidden.broadcast_mul(&mask)?.sum(1)?;
        let counts = mask.sum(1)?;
        summed.broadcast_div(&counts)?.to_vec2::<f32>()
    }
}

/// Load the model from `dir` (`config.json`, `tokenizer.json`,
/// `model.safetensors`), verifying each file against its pinned hash.
pub(crate) fn load(dir: &Path) -> Result<Model, EmbedError> {
    let config = read_verified(&dir.join("config.json"), crate::CONFIG_SHA256)?;
    let tokenizer_json = read_verified(&dir.join("tokenizer.json"), crate::TOKENIZER_SHA256)?;
    let weights = read_verified(&dir.join("model.safetensors"), crate::MODEL_SHA256)?;
    build(&config, &tokenizer_json, weights)
}

fn read_verified(path: &Path, expected: &str) -> Result<Vec<u8>, EmbedError> {
    let bytes = std::fs::read(path)?;
    let got = crate::sha256_hex(&bytes);
    if got != expected {
        return Err(EmbedError::Hash {
            file: path.display().to_string(),
            expected: expected.to_string(),
            got,
        });
    }
    Ok(bytes)
}

fn build(config: &[u8], tokenizer_json: &[u8], weights: Vec<u8>) -> Result<Model, EmbedError> {
    let device = Device::Cpu;
    let config: Config = serde_json::from_slice(config)
        .map_err(|e| EmbedError::Load(format!("parse config.json: {e}")))?;
    let mut tokenizer =
        Tokenizer::from_bytes(tokenizer_json).map_err(|e| EmbedError::Load(e.to_string()))?;
    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        ..Default::default()
    }));
    let vb = VarBuilder::from_buffered_safetensors(weights, DTYPE, &device)
        .map_err(|e| EmbedError::Load(format!("load safetensors: {e}")))?;
    let bert =
        BertModel::load(vb, &config).map_err(|e| EmbedError::Load(format!("build bert: {e}")))?;
    Ok(Model {
        bert,
        tokenizer,
        device,
    })
}
