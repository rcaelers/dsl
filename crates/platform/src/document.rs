use std::error::Error;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Failure produced by reusable host document mechanisms.
#[derive(Debug, Error)]
pub enum DocumentError {
    /// A selected document could not be read.
    #[error("could not read {}: {source}", path.display())]
    Read {
        /// Host path or opaque document reference.
        path: PathBuf,
        /// Concrete host-adapter cause.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// A selected document could not be written.
    #[error("could not write {}: {source}", path.display())]
    Write {
        /// Host path or opaque document reference.
        path: PathBuf,
        /// Concrete host-adapter cause.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// A document reference is no longer valid in the current host session.
    #[error("document '{}' is no longer available; select it again", path.display())]
    Unavailable {
        /// Expired host path or opaque reference.
        path: PathBuf,
    },
    /// A host document exceeds the bounded in-memory document capacity.
    #[error("'{name}' is too large to open ({max_bytes} byte limit)")]
    TooLarge {
        /// User-facing document name.
        name: String,
        /// Maximum supported document size.
        max_bytes: u64,
    },
    /// A destination has no parent directory to create.
    #[error("{} has no parent directory", path.display())]
    MissingParent {
        /// Destination path without a parent.
        path: PathBuf,
    },
    /// Parent-directory creation failed.
    #[error("could not create {}: {source}", path.display())]
    CreateParent {
        /// Parent directory that could not be created.
        path: PathBuf,
        /// Concrete host filesystem cause.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl DocumentError {
    pub(crate) fn write(path: &Path, error: impl Error + Send + Sync + 'static) -> Self {
        Self::Write {
            path: path.to_owned(),
            source: Box::new(error),
        }
    }
}

#[cfg(test)]
mod document_tests {
    use std::error::Error as _;
    use std::path::Path;

    use super::DocumentError;

    #[test]
    fn document_access_retains_the_host_cause() {
        let error = DocumentError::Read {
            path: Path::new("capture.json").to_owned(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "denied",
            )),
        };

        assert!(matches!(&error, DocumentError::Read { .. }));
        assert!(error.source().unwrap().is::<std::io::Error>());
    }
}
