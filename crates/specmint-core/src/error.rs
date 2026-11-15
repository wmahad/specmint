use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("source: {0}")]
    Source(String),

    #[error("adapter: {0}")]
    Adapter(String),

    #[error("emit[{emitter}]: {message}")]
    Emit {
        emitter: &'static str,
        message: String,
    },

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn source(msg: impl Into<String>) -> Self {
        Self::Source(msg.into())
    }

    pub fn adapter(msg: impl Into<String>) -> Self {
        Self::Adapter(msg.into())
    }

    pub fn emit(emitter: &'static str, message: impl Into<String>) -> Self {
        Self::Emit {
            emitter,
            message: message.into(),
        }
    }
}

/// Soft wrapper so call sites can attach path context.
#[derive(Debug)]
pub struct PathError {
    pub path: PathBuf,
    pub source: Error,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Deserialize helper used by config overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    SchemaName,
    OperationId,
}
