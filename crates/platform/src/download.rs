use std::fmt;

use thiserror::Error;

/// Host mechanism used while materializing an explicit file download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadOperation {
    /// Create the host byte payload.
    CreatePayload,
    /// Create a temporary host URL for the payload.
    CreateUrl,
    /// Access the host document.
    AccessDocument,
    /// Create the temporary download link.
    CreateLink,
    /// Access the host document body.
    AccessBody,
    /// Attach the temporary link.
    AttachLink,
    /// Remove the temporary link after activation.
    DetachLink,
    /// Release the temporary host URL.
    ReleaseUrl,
}

impl fmt::Display for DownloadOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreatePayload => "create the download payload",
            Self::CreateUrl => "create the download URL",
            Self::AccessDocument => "access the host document",
            Self::CreateLink => "create the download link",
            Self::AccessBody => "access the host document body",
            Self::AttachLink => "attach the download link",
            Self::DetachLink => "detach the download link",
            Self::ReleaseUrl => "release the download URL",
        })
    }
}

/// Failure produced by a reusable host output-download mechanism.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DownloadError {
    /// A previously listed host output is no longer queued.
    #[error("output {id} is no longer available")]
    Unavailable {
        /// Host-local output identifier.
        id: u64,
    },
    /// A host operation failed while activating a download.
    #[error("could not download '{name}': failed to {operation}: {message}")]
    Host {
        /// User-facing output filename.
        name: String,
        /// Classified host operation that failed.
        operation: DownloadOperation,
        /// Host-adapter diagnostic.
        message: String,
    },
}

#[cfg(test)]
mod download_tests {
    use super::{DownloadError, DownloadOperation};

    #[test]
    fn host_failures_retain_the_download_stage() {
        let error = DownloadError::Host {
            name: "capture.bin".to_owned(),
            operation: DownloadOperation::CreateUrl,
            message: "host rejected the payload".to_owned(),
        };

        assert!(matches!(
            error,
            DownloadError::Host {
                operation: DownloadOperation::CreateUrl,
                ..
            }
        ));
    }
}
