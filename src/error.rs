use std::io;
use std::path::PathBuf;

use thiserror::Error;
use yaml_rust2::ScanError;

/// Result type shared by the CLI, upgrade logic, and filesystem adapters.
pub type Result<T> = std::result::Result<T, FlupError>;

#[derive(Debug, Error)]
pub enum FlupError {
    #[error("{0}")]
    InvalidProject(String),
    #[error("{0}")]
    Sdk(String),
    #[error("{0}")]
    Template(String),
    #[error("{0}")]
    Dependency(String),
    #[error("{0}")]
    Commit(String),
    #[error("commit failed ({source}); {restore_status}; backup remains at {backup}")]
    CommitFailed {
        #[source]
        source: Box<FlupError>,
        restore_status: String,
        backup: PathBuf,
    },
    #[error("invalid {file} YAML: {source}")]
    InvalidYaml {
        file: &'static str,
        #[source]
        source: ScanError,
    },
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("{0}")]
    Internal(String),
}

impl FlupError {
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
