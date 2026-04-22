use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("invalid data: {0}")]
    Invalid(String),
}

pub type AppResult<T> = Result<T, AppError>;
