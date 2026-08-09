/// Failure to configure a generated collector lane through its plan-owned payload catalog.
#[derive(Debug, thiserror::Error)]
pub enum PayloadCatalogConfigurationError {
    /// The catalog implementation exposed a typed request-configuration failure.
    #[error("{source}")]
    Configuration {
        /// Concrete request-configuration cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A legacy or external catalog implementation exposed only a diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl PayloadCatalogConfigurationError {
    /// Retains a typed payload request-configuration cause.
    pub fn configuration(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Configuration {
            source: Box::new(source),
        }
    }

    /// Adapts a catalog implementation that can expose only a diagnostic.
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic(message.into())
    }
}

#[cfg(test)]
mod payload_catalog_error_tests {
    use std::error::Error;

    use super::PayloadCatalogConfigurationError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled payload request failure")]
    struct ControlledConfigurationFailure;

    #[test]
    fn typed_configuration_causes_remain_available() {
        let error = PayloadCatalogConfigurationError::configuration(ControlledConfigurationFailure);

        assert!(matches!(
            error,
            PayloadCatalogConfigurationError::Configuration { .. }
        ));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled payload request failure")
        );
    }
}
