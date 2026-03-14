//! Embeddings model pre-download.

use crate::ext::hub::DownloadRegistry;
use wcore::paths::CONFIG_DIR;
use wcore::protocol::message::DownloadKind;

const EMBEDDINGS_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";
const EMBEDDINGS_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

/// Pre-download embeddings model files into the HF cache so that
/// `Embedder::load()` finds them without network access.
///
/// Must be called before `MemoryHook::open()`. The registry tracks
/// progress and broadcasts events to any connected subscribers.
pub async fn pre_download(
    registry: &std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
) -> anyhow::Result<()> {
    let cache_dir = CONFIG_DIR.join(".cache").join("huggingface");
    let id = registry
        .lock()
        .await
        .start(DownloadKind::Embeddings, EMBEDDINGS_MODEL.into());

    let result = async {
        let api = hf_hub::api::tokio::ApiBuilder::new()
            .with_cache_dir(cache_dir)
            .with_progress(false)
            .build()?;
        let repo = api.model(EMBEDDINGS_MODEL.into());

        for filename in EMBEDDINGS_FILES {
            registry
                .lock()
                .await
                .step(id, format!("downloading {filename}..."));
            repo.get(filename).await?;
        }
        anyhow::Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            registry.lock().await.complete(id);
            Ok(())
        }
        Err(e) => {
            registry.lock().await.fail(id, e.to_string());
            Err(e)
        }
    }
}
