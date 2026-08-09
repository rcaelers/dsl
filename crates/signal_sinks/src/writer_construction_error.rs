/// Failure to construct a portable output-writer processing node.
#[derive(Debug, thiserror::Error)]
pub enum WriterConstructionError {
    /// The requested writer configuration is invalid.
    #[error("{0}")]
    Configuration(String),
    /// A writer implementation exposed a typed construction failure.
    #[error("{source}")]
    Construction {
        /// Concrete writer-construction cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A legacy or external writer implementation exposed only a diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl WriterConstructionError {
    /// Classifies an invalid writer configuration.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    /// Retains a typed writer-construction cause.
    pub fn construction(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Construction {
            source: Box::new(source),
        }
    }

    /// Adapts an implementation that can expose only a diagnostic.
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic(message.into())
    }
}

#[cfg(test)]
mod writer_construction_error_tests {
    use std::error::Error;

    use super::WriterConstructionError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled writer construction failure")]
    struct ControlledConstructionFailure;

    #[test]
    fn typed_writer_construction_causes_remain_available() {
        let error = WriterConstructionError::construction(ControlledConstructionFailure);

        assert!(matches!(
            error,
            WriterConstructionError::Construction { .. }
        ));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled writer construction failure")
        );
    }
}
