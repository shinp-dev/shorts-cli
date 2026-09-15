use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum VedError {
    #[error("{0}")]
    Message(String),
    #[error("{message}")]
    Coded { code: &'static str, message: String },
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{program} could not be started: {source}")]
    Process {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} failed with exit code {code}: {stderr}")]
    ProcessFailed {
        program: String,
        code: String,
        stderr: String,
    },
}

pub type Result<T> = std::result::Result<T, VedError>;

pub fn message(value: impl Into<String>) -> VedError {
    VedError::Message(value.into())
}

pub fn coded(code: &'static str, value: impl Into<String>) -> VedError {
    VedError::Coded {
        code,
        message: value.into(),
    }
}
