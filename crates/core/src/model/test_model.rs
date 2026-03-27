//! Mock model for testing — returns scripted responses in sequence.
//!
//! `TestModel` implements `Model` with a pre-loaded sequence of responses.
//! Each `send()` call pops the next `Response`; each `stream()` call pops
//! the next `Vec<StreamChunk>` and yields them one at a time. Both panic
//! if called more times than scripted.

use super::{Request, Response, StreamChunk};
use anyhow::Result;
use futures_core::Stream;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// A mock model that returns scripted responses in order.
///
/// Thread-safe (uses `Arc<Mutex<_>>`) and `Clone` as required by `Model`.
/// Fails with an error if more calls are made than scripted responses.
#[derive(Clone)]
pub struct TestModel {
    responses: Arc<Mutex<VecDeque<Response>>>,
    chunks: Arc<Mutex<VecDeque<Vec<StreamChunk>>>>,
    model_name: String,
}

impl TestModel {
    /// Create a new test model with scripted send() responses.
    pub fn new(responses: Vec<Response>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            chunks: Arc::new(Mutex::new(VecDeque::new())),
            model_name: "test-model".into(),
        }
    }

    /// Create a new test model with scripted stream() chunk sequences.
    pub fn with_chunks(chunks: Vec<Vec<StreamChunk>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            chunks: Arc::new(Mutex::new(chunks.into())),
            model_name: "test-model".into(),
        }
    }

    /// Create a test model with both send and stream responses.
    pub fn with_both(responses: Vec<Response>, chunks: Vec<Vec<StreamChunk>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            chunks: Arc::new(Mutex::new(chunks.into())),
            model_name: "test-model".into(),
        }
    }
}

impl super::Model for TestModel {
    async fn send(&self, _request: &Request) -> Result<Response> {
        let mut responses = self.responses.lock().unwrap();
        responses
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("TestModel: no more scripted responses for send()"))
    }

    fn stream(&self, _request: Request) -> impl Stream<Item = Result<StreamChunk>> + Send {
        let chunks = {
            let mut all = self.chunks.lock().unwrap();
            all.pop_front()
        };
        async_stream::stream! {
            match chunks {
                Some(chunks) => {
                    for chunk in chunks {
                        yield Ok(chunk);
                    }
                }
                None => {
                    yield Err(anyhow::anyhow!("TestModel: no more scripted chunks for stream()"));
                }
            }
        }
    }

    fn active_model(&self) -> String {
        self.model_name.clone()
    }
}
