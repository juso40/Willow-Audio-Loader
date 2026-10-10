//! The error type that the format parsers and in-memory bank builders share.

use std::fmt;
use std::path::PathBuf;

/// Everything that can go wrong reading containers or crafting audio in memory.
#[derive(Debug)]
pub enum AudioError {
    /// Filesystem failure, with the path it happened on.
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The operating system's error.
        source: std::io::Error,
    },
    /// The bytes are not the container or codec we expect. The message
    /// names exactly what was wrong (offsets are hex, counts decimal).
    Format(String),
    /// A requested media id is not in the target file.
    NotFound(String),
}

impl AudioError {
    /// Convenience constructor for [`AudioError::Format`].
    pub fn format(message: impl Into<String>) -> Self {
        Self::Format(message.into())
    }

    /// Convenience constructor for [`AudioError::NotFound`].
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Format(message) | Self::NotFound(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Format(_) | Self::NotFound(_) => None,
        }
    }
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, AudioError>;
