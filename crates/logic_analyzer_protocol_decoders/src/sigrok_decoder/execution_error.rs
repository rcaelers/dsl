use std::fmt;

/// Operation performed through a running Sigrok execution host port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigrokExecutionOperation {
    /// Submission of logic or protocol input.
    Input,
    /// Retrieval or conversion of decoder output.
    Output,
    /// End-of-input signaling and decoder flushing.
    Completion,
    /// Worker shutdown and result collection.
    Join,
}

impl fmt::Display for SigrokExecutionOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Completion => "completion",
            Self::Join => "join",
        };
        formatter.write_str(name)
    }
}

/// Failure to start a host-provided Sigrok execution.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SigrokExecutionStartError {
    /// The host does not provide Sigrok execution.
    #[error("{0}")]
    Unavailable(String),
    /// The host retained a typed worker-start cause.
    #[error("could not start Sigrok execution: {source}")]
    Startup {
        /// Concrete worker-start cause.
        #[source]
        source: std::sync::Arc<dyn std::error::Error + Send + Sync>,
    },
    /// A legacy or external host exposed only a startup diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl SigrokExecutionStartError {
    /// Classifies a host without Sigrok execution support.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Retains a typed worker-start cause.
    pub fn startup(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Startup {
            source: std::sync::Arc::new(source),
        }
    }

    /// Adapts a host that can expose only a startup diagnostic.
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic(message.into())
    }
}

/// Failure reported by a running host-provided Sigrok execution.
#[derive(Debug, thiserror::Error)]
pub enum SigrokExecutionError {
    /// Input submission failed.
    #[error("Sigrok execution input failed: {source}")]
    Input {
        /// Concrete input transport or conversion cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Output retrieval or conversion failed.
    #[error("Sigrok execution output failed: {source}")]
    Output {
        /// Concrete output transport or conversion cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// End-of-input signaling or decoder flushing failed.
    #[error("Sigrok execution completion failed: {source}")]
    Completion {
        /// Concrete completion cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Worker shutdown or result collection failed.
    #[error("Sigrok execution join failed: {source}")]
    Join {
        /// Concrete worker-join cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A legacy or external execution exposed only a diagnostic.
    #[error("Sigrok execution {operation} failed: {message}")]
    Diagnostic {
        /// Operation that failed.
        operation: SigrokExecutionOperation,
        /// Diagnostic supplied by the execution.
        message: String,
    },
}

impl SigrokExecutionError {
    /// Retains a typed input transport or conversion cause.
    pub fn input(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Input {
            source: Box::new(source),
        }
    }

    /// Retains a typed output transport or conversion cause.
    pub fn output(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Output {
            source: Box::new(source),
        }
    }

    /// Retains a typed completion cause.
    pub fn completion(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Completion {
            source: Box::new(source),
        }
    }

    /// Retains a typed worker-join cause.
    pub fn join(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Join {
            source: Box::new(source),
        }
    }

    /// Adapts an execution that can expose only a diagnostic.
    pub fn diagnostic(operation: SigrokExecutionOperation, message: impl Into<String>) -> Self {
        Self::Diagnostic {
            operation,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod execution_error_tests {
    use std::error::Error;

    use super::{SigrokExecutionError, SigrokExecutionStartError};

    #[derive(Debug, thiserror::Error)]
    #[error("controlled host failure")]
    struct ControlledHostFailure;

    #[test]
    fn typed_start_and_operation_causes_remain_available() {
        let start = SigrokExecutionStartError::startup(ControlledHostFailure);
        let output = SigrokExecutionError::output(ControlledHostFailure);

        assert_eq!(
            start.source().map(ToString::to_string).as_deref(),
            Some("controlled host failure")
        );
        assert_eq!(
            output.source().map(ToString::to_string).as_deref(),
            Some("controlled host failure")
        );
    }
}
