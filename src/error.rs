//! Custom error types for ditrive

use thiserror::Error;

/// Main error type for ditrive operations
#[derive(Error, Debug)]
pub enum DitriveError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Directory walk error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("Google Drive error: {0}")]
    Drive(String),

    #[error("GitHub error: {0}")]
    GitHub(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Not a git repository: {0}")]
    NotGitRepo(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("Retry exhausted after {attempts} attempts: {message}")]
    RetryExhausted { attempts: u32, message: String },
}

pub type Result<T> = std::result::Result<T, DitriveError>;
