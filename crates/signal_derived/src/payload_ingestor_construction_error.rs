/// Failure to construct retained-lane ingestion for a registered payload.
#[derive(Debug, thiserror::Error)]
pub enum PayloadIngestorConstructionError {
    /// The collected-lane request is incompatible with the payload adapter.
    #[error("{0}")]
    Configuration(String),
    /// A payload adapter exposed a typed construction failure.
    #[error("{source}")]
    Construction {
        /// Concrete payload-ingestor construction cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A legacy or external payload adapter exposed only a diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl PayloadIngestorConstructionError {
    /// Classifies an invalid collected-lane request.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    /// Retains a typed payload-ingestor construction cause.
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
mod payload_ingestor_construction_error_tests {
    use std::error::Error;

    use super::PayloadIngestorConstructionError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled payload-ingestor failure")]
    struct ControlledConstructionFailure;

    #[test]
    fn typed_ingestor_causes_remain_available() {
        let error = PayloadIngestorConstructionError::construction(ControlledConstructionFailure);

        assert!(matches!(
            error,
            PayloadIngestorConstructionError::Construction { .. }
        ));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled payload-ingestor failure")
        );
    }
}
