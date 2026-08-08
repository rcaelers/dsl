use std::fmt;

use thiserror::Error;

use platform_runtime::WorkerQueueError;

/// Host operation performed while constructing a worker adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerAdapterOperation {
    /// Create the host payload containing the worker bootstrap.
    CreateBootstrapPayload,
    /// Create a temporary host URL for the worker bootstrap.
    CreateBootstrapUrl,
    /// Start one worker from the temporary bootstrap URL.
    StartWorker,
    /// Release the temporary worker bootstrap URL.
    ReleaseBootstrapUrl,
}

impl fmt::Display for WorkerAdapterOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreateBootstrapPayload => "create the worker bootstrap payload",
            Self::CreateBootstrapUrl => "create the worker bootstrap URL",
            Self::StartWorker => "start a worker",
            Self::ReleaseBootstrapUrl => "release the worker bootstrap URL",
        })
    }
}

/// Failure while constructing a reusable host worker adapter.
#[derive(Debug, Error)]
pub enum WorkerAdapterError {
    /// The portable bounded worker queue rejected its configuration.
    #[error("invalid worker-adapter configuration: {source}")]
    Queue {
        /// Portable queue configuration cause.
        #[source]
        source: WorkerQueueError,
    },
    /// A native host thread could not be started.
    #[error("could not start native worker {worker_index}: {source}")]
    NativeWorkerStart {
        /// Zero-based worker slot that could not be started.
        worker_index: usize,
        /// Native thread creation cause.
        #[source]
        source: std::io::Error,
    },
    /// A host mechanism failed while creating a worker pool.
    #[error("failed to {operation}: {message}")]
    Host {
        /// Classified host construction operation.
        operation: WorkerAdapterOperation,
        /// Host-adapter diagnostic.
        message: String,
    },
}

impl From<WorkerQueueError> for WorkerAdapterError {
    fn from(source: WorkerQueueError) -> Self {
        Self::Queue { source }
    }
}

#[cfg(test)]
mod worker_adapter_tests {
    use std::error::Error as _;

    use platform_runtime::WorkerQueueError;

    use super::WorkerAdapterError;

    #[test]
    fn queue_configuration_retains_the_portable_cause() {
        let error = WorkerAdapterError::from(WorkerQueueError::EmptyPool);

        assert!(error.source().unwrap().is::<WorkerQueueError>());
    }

    #[test]
    fn native_startup_retains_the_io_cause() {
        let error = WorkerAdapterError::NativeWorkerStart {
            worker_index: 3,
            source: std::io::Error::new(std::io::ErrorKind::ResourceBusy, "busy"),
        };

        assert!(error.source().unwrap().is::<std::io::Error>());
    }
}
