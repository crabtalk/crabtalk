use thiserror::Error;

/// Top-level error type for the meta search engine.
#[derive(Error, Debug)]
pub enum Error {
    #[error("engine error ({engine}): {message}")]
    Engine { engine: String, message: String },

    #[error("configuration error: {0}")]
    Config(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("no engines configured")]
    NoEngines,
}

/// Error type for individual engine failures.
#[derive(Error, Debug)]
pub enum EngineError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("rate limited")]
    RateLimited,

    #[error("{0}")]
    Other(String),
}
