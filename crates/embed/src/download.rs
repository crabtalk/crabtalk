//! Runtime model provisioning for the `download` feature: fetch the pinned model
//! files into a cache dir (skipping any already present with the right hash),
//! verify each against its pinned sha256, and hand back the dir for
//! [`ModelSource::Dir`](crate::ModelSource::Dir). Same bytes as the bundled
//! model → identical vectors.
use crate::{EmbedError, sha256_hex};
use std::path::{Path, PathBuf};

/// Where the pinned files are fetched from. Override per file if you mirror the
/// weights; the sha256 check rejects anything that isn't the pinned model.
const BASE: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main";

/// `(filename, pinned sha256, url)` for the three model files.
fn files() -> [(&'static str, &'static str, String); 3] {
    [
        ("config.json", crate::CONFIG_SHA256, format!("{BASE}/config.json")),
        (
            "tokenizer.json",
            crate::TOKENIZER_SHA256,
            format!("{BASE}/tokenizer.json"),
        ),
        (
            "model.safetensors",
            crate::MODEL_SHA256,
            format!("{BASE}/model.safetensors"),
        ),
    ]
}

/// Ensure the pinned model files exist and are correct under `dir`, downloading
/// the missing/mismatched ones, then return `dir` for `ModelSource::Dir(dir)`.
pub async fn ensure(dir: impl AsRef<Path>) -> Result<PathBuf, EmbedError> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)?;
    let client = reqwest::Client::new();
    for (name, expected, url) in files() {
        let path = dir.join(name);
        if path.exists() && sha256_hex(&std::fs::read(&path)?) == expected {
            continue;
        }
        let resp = client
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|source| EmbedError::Download {
                url: url.clone(),
                source,
            })?;
        let bytes = resp.bytes().await.map_err(|source| EmbedError::Download {
            url: url.clone(),
            source,
        })?;
        let got = sha256_hex(&bytes);
        if got != expected {
            return Err(EmbedError::Hash {
                file: name.to_string(),
                expected: expected.to_string(),
                got,
            });
        }
        std::fs::write(&path, &bytes)?;
    }
    Ok(dir.to_path_buf())
}
