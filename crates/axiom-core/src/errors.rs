use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AxiomError>;

#[derive(Debug, Error)]
pub enum AxiomError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse TOML: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    #[error("failed to serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("could not determine the platform config directory")]
    MissingConfigDirectory,
    #[error(
        "config schema version {found} is newer than this Axiom build supports ({supported}). Update Axiom before using this config."
    )]
    UnsupportedConfigVersion { found: u32, supported: u32 },
    #[error("invalid config value for `{field}`: {message}")]
    InvalidConfig {
        field: &'static str,
        message: String,
    },
    #[error("path is outside the workspace: {path}")]
    UnsafeWorkspacePath { path: PathBuf },
    #[error("path cannot be normalized safely: {path}")]
    InvalidPath { path: PathBuf },
}
