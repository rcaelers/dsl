use logic_analyzer_acquisition::LogicAnalyzerError;

/// Failure to construct a DSLogic U3Pro16 processing source.
#[derive(Debug, thiserror::Error)]
pub enum DsLogicU3Pro16SourceError {
    /// The host did not supply a compatible device capability.
    #[error("{0}")]
    Unavailable(String),
    /// Device transport or source initialization failed.
    #[error("could not initialize DSLogic U3Pro16 source: {0}")]
    Acquisition(#[source] LogicAnalyzerError),
    /// An external source implementation exposed a typed construction failure.
    #[error("{source}")]
    Construction {
        /// Concrete source-construction cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A legacy or external source implementation exposed only a diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl DsLogicU3Pro16SourceError {
    /// Classifies an unavailable host device capability.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Retains a typed external source-construction cause.
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

impl From<LogicAnalyzerError> for DsLogicU3Pro16SourceError {
    fn from(source: LogicAnalyzerError) -> Self {
        Self::Acquisition(source)
    }
}

#[cfg(test)]
mod dslogic_source_error_tests {
    use std::error::Error;

    use logic_analyzer_acquisition::LogicAnalyzerError;

    use super::DsLogicU3Pro16SourceError;

    #[test]
    fn acquisition_causes_remain_available() {
        let error = DsLogicU3Pro16SourceError::from(LogicAnalyzerError::NotCapturing);

        assert!(matches!(error, DsLogicU3Pro16SourceError::Acquisition(_)));
        assert!(error.source().is_some());
    }
}
