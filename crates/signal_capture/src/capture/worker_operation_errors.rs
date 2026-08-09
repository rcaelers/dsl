use platform_runtime::WorkerOperation;

/// Failure while configuring the capture-worker operation inventory.
#[derive(Debug, thiserror::Error)]
pub enum CaptureWorkerOperationRegistrationError {
    /// More than one handler claimed the same stable operation identifier.
    #[error(
        "capture-worker operation '{operation}' is already registered",
        operation = .operation.as_str()
    )]
    Duplicate {
        /// Operation identifier claimed more than once.
        operation: WorkerOperation,
    },
}

/// Failure while selecting or invoking one capture preparation operation.
#[derive(Debug, thiserror::Error)]
pub enum CaptureWorkerOperationPreparationError {
    /// No handler was registered for the requested operation identifier.
    #[error(
        "capture-worker operation '{operation}' is not registered",
        operation = .operation.as_str()
    )]
    Unregistered {
        /// Operation identifier requested by the worker client.
        operation: WorkerOperation,
    },
    /// A registered handler could not prepare its capture index.
    #[error(
        "capture-worker operation '{operation}' could not prepare its index: {source}",
        operation = .operation.as_str()
    )]
    Handler {
        /// Operation identifier whose handler failed.
        operation: WorkerOperation,
        /// Concrete handler-owned preparation failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl CaptureWorkerOperationPreparationError {
    pub(crate) fn handler(
        operation: WorkerOperation,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Handler {
            operation,
            source: Box::new(source),
        }
    }
}

#[cfg(test)]
mod worker_operation_error_tests {
    use std::error::Error;

    use platform_runtime::WorkerOperation;

    use super::CaptureWorkerOperationPreparationError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled preparation failure")]
    struct ControlledPreparationFailure;

    #[test]
    fn handler_causes_remain_available() {
        let error = CaptureWorkerOperationPreparationError::handler(
            WorkerOperation::new("org.example.capture.prepare/v1").unwrap(),
            ControlledPreparationFailure,
        );

        assert!(matches!(
            error,
            CaptureWorkerOperationPreparationError::Handler { .. }
        ));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled preparation failure")
        );
    }
}
