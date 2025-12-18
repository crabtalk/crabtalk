//! Cydonia processor config

use crate::Processor;
use candle_core::Device;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Processor configuration
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct ProcessorConfig {
    /// If running the model with gpu
    pub gpu: bool,

    /// The seed of the model
    pub seed: Option<u64>,

    /// The temperature of the model
    pub temperature: Option<f64>,

    /// The top-p of the model
    pub top_p: Option<f64>,

    /// The top-k of the model
    pub top_k: Option<usize>,

    /// The repeat penalty of the model
    pub repeat_penalty: f32,

    /// The repeat last n of the model
    pub repeat_last_n: usize,

    /// The sample length of the model
    pub sample_len: usize,
}

impl ProcessorConfig {
    /// Build the processor
    pub fn build(self) -> Processor {
        let temperature = self.temperature.unwrap_or(0.6);
        let sampling = if temperature <= 0. {
            Sampling::ArgMax
        } else {
            match (self.top_k, self.top_p) {
                (None, None) => Sampling::All { temperature },
                (Some(k), None) => Sampling::TopK { k, temperature },
                (None, Some(p)) => Sampling::TopP { p, temperature },
                (Some(k), Some(p)) => Sampling::TopKThenTopP { k, p, temperature },
            }
        };

        let seed = self.seed.unwrap_or_else(|| rand::rng().random::<u64>());
        Processor {
            processor: LogitsProcessor::from_sampling(seed, sampling),
            device: crate::device::detect(!self.gpu).unwrap_or(Device::Cpu),
            repeat_penalty: self.repeat_penalty,
            repeat_last_n: self.repeat_last_n,
            sample_len: self.sample_len,
        }
    }

    /// Set the gpu
    pub fn gpu(mut self, gpu: bool) -> Self {
        self.gpu = gpu;
        self
    }

    /// Set the temperature
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the top-p
    pub fn top_p(mut self, top_p: f64) -> Self {
        self.top_p = Some(top_p);
        self
    }

    /// Set the top-k
    pub fn top_k(mut self, top_k: usize) -> Self {
        self.top_k = Some(top_k);
        self
    }

    /// Set the seed
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set the repeat penalty
    pub fn repeat_penalty(mut self, repeat_penalty: f32) -> Self {
        self.repeat_penalty = repeat_penalty;
        self
    }

    /// Set the repeat last n
    pub fn repeat_last_n(mut self, repeat_last_n: usize) -> Self {
        self.repeat_last_n = repeat_last_n;
        self
    }

    /// Set the sample length
    ///
    /// TODO: if there is a way to embed the sample length in the system prompt?
    pub fn sample_len(mut self, sample_len: usize) -> Self {
        self.sample_len = sample_len;
        self
    }
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            gpu: false,
            seed: None,
            temperature: Some(0.6),
            top_p: Some(0.9),
            top_k: Some(50),
            sample_len: 1024,
            repeat_penalty: 1.0,
            repeat_last_n: 64,
        }
    }
}
