use std::fmt;

use thiserror::Error;

use platform_artifacts::RepositoryError;

/// Host operation performed while opening a durable artifact repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRepositoryOpenOperation {
    /// Create the host payload containing the persistence-worker bootstrap.
    CreateWorkerPayload,
    /// Create a temporary host URL for the persistence-worker bootstrap.
    CreateWorkerUrl,
    /// Start the persistence worker.
    StartWorker,
    /// Release the temporary persistence-worker URL.
    ReleaseWorkerUrl,
    /// Send the repository initialization request to the worker.
    SendInitialization,
    /// Await worker initialization or a host startup failure.
    AwaitInitialization,
    /// Start the host pump that submits queued persistence commands.
    StartCommandPump,
}

impl fmt::Display for ArtifactRepositoryOpenOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreateWorkerPayload => "create the persistence-worker bootstrap payload",
            Self::CreateWorkerUrl => "create the persistence-worker bootstrap URL",
            Self::StartWorker => "start the persistence worker",
            Self::ReleaseWorkerUrl => "release the persistence-worker bootstrap URL",
            Self::SendInitialization => "send the repository initialization request",
            Self::AwaitInitialization => "await repository initialization",
            Self::StartCommandPump => "start the persistence command pump",
        })
    }
}

/// Failure while opening a reusable host artifact repository.
#[derive(Debug, Error)]
pub enum ArtifactRepositoryOpenError {
    /// The requested host repository namespace is invalid.
    #[error("the artifact repository root name must not be empty")]
    InvalidRootName,
    /// The host cannot provide durable artifact persistence.
    #[error("durable artifact persistence is unavailable: {message}")]
    Unavailable {
        /// Host-provided availability diagnostic.
        message: String,
    },
    /// The persistence worker returned an invalid initialization response.
    #[error("invalid artifact repository initialization response: {message}")]
    Protocol {
        /// Response validation diagnostic.
        message: String,
    },
    /// A host mechanism failed while opening the repository.
    #[error("failed to {operation}: {message}")]
    Host {
        /// Classified host construction operation.
        operation: ArtifactRepositoryOpenOperation,
        /// Host-adapter diagnostic.
        message: String,
    },
    /// A durable entry could not be copied into the session repository.
    #[error("could not hydrate the session artifact repository: {source}")]
    Hydration {
        /// Portable repository failure raised while restoring an entry.
        #[source]
        source: RepositoryError,
    },
}

#[cfg(test)]
mod artifact_repository_tests {
    use std::error::Error as _;

    use platform_artifacts::RepositoryError;

    use super::{ArtifactRepositoryOpenError, ArtifactRepositoryOpenOperation};

    #[test]
    fn hydration_retains_the_portable_repository_cause() {
        let error = ArtifactRepositoryOpenError::Hydration {
            source: RepositoryError::QuotaExceeded,
        };

        assert!(error.source().unwrap().is::<RepositoryError>());
    }

    #[test]
    fn host_failures_retain_the_open_stage() {
        let error = ArtifactRepositoryOpenError::Host {
            operation: ArtifactRepositoryOpenOperation::StartWorker,
            message: "worker rejected".to_owned(),
        };

        assert!(matches!(
            error,
            ArtifactRepositoryOpenError::Host {
                operation: ArtifactRepositoryOpenOperation::StartWorker,
                ..
            }
        ));
    }
}
