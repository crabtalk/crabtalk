//! Model file download operation.

use crate::ext::hub::DownloadRegistry;
use compact_str::CompactString;
use wcore::protocol::message::server::{DownloadEvent, DownloadKind};

/// Download a model's files, streaming unified download events.
///
/// Registers the download in the registry, delegates to
/// `model::local::download::download_model()`, and converts internal
/// progress events into unified `DownloadEvent`s.
pub fn download(
    model: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
) -> impl futures_core::Stream<Item = anyhow::Result<DownloadEvent>> {
    async_stream::try_stream! {
        let id = registry
            .lock()
            .await
            .start(DownloadKind::Model, model.to_string());
        yield DownloadEvent::Created {
            id,
            kind: DownloadKind::Model,
            label: model.to_string(),
        };

        #[cfg(feature = "local")]
        {
            let entry = model::local::registry::find(&model)
                .ok_or_else(|| anyhow::anyhow!(
                    "model '{}' is not in the registry", model
                ))?;

            if !entry.fits() {
                let required = entry.memory_requirement();
                let actual = model::local::system_memory() / (1024 * 1024 * 1024);
                let err = format!(
                    "model '{}' requires at least {} RAM, your system has {}GB",
                    entry.name, required, actual
                );
                registry.lock().await.fail(id, err.clone());
                Err(anyhow::anyhow!("{err}"))?;
            }

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let model_str = model.to_string();
            let download_handle = tokio::spawn(async move {
                model::local::download::download_model(&model_str, tx).await
            });

            while let Some(event) = rx.recv().await {
                match event {
                    model::local::download::DownloadEvent::FileStart { filename, size } => {
                        let msg = format!("downloading {filename} ({size} bytes)");
                        registry.lock().await.step(id, msg.clone());
                        yield DownloadEvent::Step { id, message: msg };
                    }
                    model::local::download::DownloadEvent::Progress { bytes } => {
                        registry.lock().await.progress(id, bytes, 0);
                        yield DownloadEvent::Progress { id, bytes, total_bytes: 0 };
                    }
                    model::local::download::DownloadEvent::FileEnd { filename } => {
                        let msg = format!("{filename} done");
                        registry.lock().await.step(id, msg.clone());
                        yield DownloadEvent::Step { id, message: msg };
                    }
                }
            }

            match download_handle.await {
                Ok(Ok(())) => {
                    registry.lock().await.complete(id);
                    yield DownloadEvent::Completed { id };
                }
                Ok(Err(e)) => {
                    let err = format!("download failed: {e}");
                    registry.lock().await.fail(id, err.clone());
                    Err(anyhow::anyhow!("{err}"))?;
                }
                Err(e) => {
                    let err = format!("download task panicked: {e}");
                    registry.lock().await.fail(id, err.clone());
                    Err(anyhow::anyhow!("{err}"))?;
                }
            }
        }

        #[cfg(not(feature = "local"))]
        {
            let err = "this daemon was built without local model support".to_string();
            registry.lock().await.fail(id, err.clone());
            Err(anyhow::anyhow!("{err}"))?;
        }
    }
}
