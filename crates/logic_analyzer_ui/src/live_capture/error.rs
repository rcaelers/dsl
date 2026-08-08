//! Typed failures for application-level live-capture coordination.

use thiserror::Error;

use logic_analyzer_graph_capabilities::node::CaptureGraphSourceError;
use platform_artifacts::RepositoryError;
use platform_runtime::WorkExecutorError;
use signal_capture_session::{AcquisitionError, CapturePolicyError, CaptureStoreError};

use crate::capture_export_service::CaptureExportServiceError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureRepositoryOperation {
    BuildApplicationMetadataKey,
    #[cfg(test)]
    ReadApplicationMetadata,
    WriteApplicationMetadata,
}

impl std::fmt::Display for CaptureRepositoryOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::BuildApplicationMetadataKey => "build capture application metadata key",
            #[cfg(test)]
            Self::ReadApplicationMetadata => "read capture application metadata",
            Self::WriteApplicationMetadata => "write capture application metadata",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureStoreOperation {
    ReserveSession,
    CreateStore,
    WriteTimelineMetadata,
    WriteSessionPlan,
    OpenAnalysisCursor,
    OpenReplayCursor,
    PinWaveform,
    DiscardSession,
    FinalizeSession,
}

impl std::fmt::Display for CaptureStoreOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ReserveSession => "reserve capture session",
            Self::CreateStore => "create capture store",
            Self::WriteTimelineMetadata => "write capture timeline metadata",
            Self::WriteSessionPlan => "write capture session plan",
            Self::OpenAnalysisCursor => "open live-analysis cursor",
            Self::OpenReplayCursor => "open finalized capture cursor",
            Self::PinWaveform => "pin capture waveform",
            Self::DiscardSession => "discard capture session",
            Self::FinalizeSession => "finalize capture session",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureAttachmentKind {
    LiveAnalysis,
    Replay,
}

impl std::fmt::Display for CaptureAttachmentKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::LiveAnalysis => "live-analysis",
            Self::Replay => "capture-replay",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureWaveformOperation {
    Build,
    FinishPrevious,
    Rebuild,
}

impl std::fmt::Display for CaptureWaveformOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Build => "build capture waveform",
            Self::FinishPrevious => "finish previous capture waveform",
            Self::Rebuild => "rebuild capture waveform",
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum CaptureCoordinatorError {
    #[error("{0}")]
    Policy(String),
    #[error("capture workflow protocol failed: {0}")]
    Protocol(String),
    #[error("could not {operation}: {source}")]
    Repository {
        operation: CaptureRepositoryOperation,
        #[source]
        source: RepositoryError,
    },
    #[error("could not {operation}: {source}")]
    Store {
        operation: CaptureStoreOperation,
        #[source]
        source: CaptureStoreError,
    },
    #[error("could not build {attachment} source: {source}")]
    GraphSource {
        attachment: CaptureAttachmentKind,
        #[source]
        source: CaptureGraphSourceError,
    },
    #[error("could not {operation}: {source}")]
    Waveform {
        operation: CaptureWaveformOperation,
        #[source]
        source: signal_capture::Error,
    },
    #[error("could not encode capture application metadata: {0}")]
    MetadataEncode(#[source] serde_json::Error),
    #[error("invalid capture application metadata: {0}")]
    #[cfg(test)]
    MetadataDecode(#[source] serde_json::Error),
    #[error("capture export failed: {0}")]
    Export(#[source] CaptureExportServiceError),
    #[error("could not start capture supervisor: {0}")]
    Executor(#[source] WorkExecutorError),
    #[error("capture acquisition failed: {0}")]
    Acquisition(#[source] AcquisitionError),
    #[error("capture policy failed: {0}")]
    CapturePolicy(#[source] CapturePolicyError),
}

impl CaptureCoordinatorError {
    pub(crate) fn policy(message: impl Into<String>) -> Self {
        Self::Policy(message.into())
    }

    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }

    pub(crate) fn repository(
        operation: CaptureRepositoryOperation,
        source: RepositoryError,
    ) -> Self {
        Self::Repository { operation, source }
    }

    pub(crate) fn store(operation: CaptureStoreOperation, source: CaptureStoreError) -> Self {
        Self::Store { operation, source }
    }

    pub(crate) fn graph_source(
        attachment: CaptureAttachmentKind,
        source: CaptureGraphSourceError,
    ) -> Self {
        Self::GraphSource { attachment, source }
    }

    pub(crate) fn waveform(
        operation: CaptureWaveformOperation,
        source: signal_capture::Error,
    ) -> Self {
        Self::Waveform { operation, source }
    }
}

#[cfg(test)]
mod error_tests {
    use std::error::Error as StdError;

    use logic_analyzer_graph_capabilities::node::CaptureGraphSourceError;
    use signal_capture_session::CaptureValidationError;

    use super::{CaptureAttachmentKind, CaptureCoordinatorError};

    #[test]
    fn graph_attachment_retains_the_session_validation_cause() {
        let error = CaptureCoordinatorError::graph_source(
            CaptureAttachmentKind::LiveAnalysis,
            CaptureGraphSourceError::new(CaptureValidationError::AnalysisChannelsEmpty),
        );

        let graph_source = StdError::source(&error)
            .and_then(|source| source.downcast_ref::<CaptureGraphSourceError>())
            .expect("graph-source cause");
        assert!(
            StdError::source(graph_source)
                .and_then(|source| source.downcast_ref::<CaptureValidationError>())
                .is_some()
        );
    }
}
