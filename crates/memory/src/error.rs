use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("entry not found: {0}")]
    NotFound(String),
    #[error("entry already exists: {0}")]
    Duplicate(String),
    #[error("bad memory file format: {0}")]
    BadFormat(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
