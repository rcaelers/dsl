use platform_artifacts::SourceReadError;
use signal_capture::Error as CaptureError;

/// Failure to construct a replay source for a prepared capture file.
#[derive(Debug, thiserror::Error)]
pub enum CaptureSourceConstructionError {
    /// The host did not supply acquisition for the configured file source.
    #[error("{0}")]
    Unavailable(String),
    /// The prepared byte source could not be acquired.
    #[error("could not access prepared capture source: {0}")]
    SourceAccess(#[source] SourceReadError),
    /// Capture-format parsing or source initialization failed.
    #[error("could not initialize capture source: {0}")]
    Capture(#[source] CaptureError),
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

impl CaptureSourceConstructionError {
    /// Classifies a host capability that is unavailable by construction.
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

impl From<SourceReadError> for CaptureSourceConstructionError {
    fn from(source: SourceReadError) -> Self {
        Self::SourceAccess(source)
    }
}

impl From<CaptureError> for CaptureSourceConstructionError {
    fn from(source: CaptureError) -> Self {
        Self::Capture(source)
    }
}

#[cfg(test)]
mod source_construction_error_tests {
    use std::error::Error;

    use platform_artifacts::SourceReadError;

    use super::CaptureSourceConstructionError;

    #[test]
    fn prepared_source_access_causes_remain_available() {
        let error = CaptureSourceConstructionError::from(SourceReadError::SourceChanged);

        assert!(matches!(
            error,
            CaptureSourceConstructionError::SourceAccess(_)
        ));
        assert!(error.source().is_some());
    }
}
