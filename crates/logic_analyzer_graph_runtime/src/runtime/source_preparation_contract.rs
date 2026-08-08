use std::sync::Arc;

use thiserror::Error;

use logic_analyzer_graph_capabilities::node_support::CapturePresentationSignal;
use logic_analyzer_graph_plan::CapturePresentationDiscoveryError;
use platform_runtime::WorkExecutorError;
use signal_capture::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureMetadata, CaptureWorkerClientError,
    CaptureWorkerFailure, CaptureWorkerMessageKind,
};

/// Invalid capture-worker response received while preparing a finite source.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SourcePreparationProtocolError {
    /// The worker returned an update reserved for another request kind.
    #[error("capture worker returned {received} data for a preparation request")]
    UnexpectedResponse {
        /// Kind of response that violated the preparation protocol.
        received: CaptureWorkerMessageKind,
    },
}

/// Failure produced while discovering or preparing one finite capture source.
#[derive(Clone, Debug, Error)]
pub enum SourcePreparationError {
    /// The graph's finite-source presentation contract could not be discovered.
    #[error("capture-source discovery failed: {0}")]
    Discovery(#[source] CapturePresentationDiscoveryError),
    /// Source metadata could not be inspected before preparation.
    #[error("capture metadata inspection failed: {0}")]
    Metadata(#[source] Arc<signal_capture::Error>),
    /// The source's waveform index could not be opened or built.
    #[error("capture index preparation failed: {0}")]
    Index(#[source] Arc<signal_capture::Error>),
    /// The current preparation generation was cancelled.
    #[error("source preparation was cancelled")]
    Cancelled,
    /// The injected preparation executor rejected or lost the operation.
    #[error("capture preparation executor failed: {0}")]
    Executor(#[source] WorkExecutorError),
    /// The bounded capture-worker client rejected the request.
    #[error("capture preparation worker client failed: {0}")]
    WorkerClient(#[source] CaptureWorkerClientError),
    /// The capture worker reported a classified terminal failure.
    #[error("capture preparation worker failed: {0}")]
    Worker(#[source] CaptureWorkerFailure),
    /// The host worker returned a response that does not belong to preparation.
    #[error("capture preparation worker protocol failed: {0}")]
    WorkerProtocol(#[source] SourcePreparationProtocolError),
}

impl SourcePreparationError {
    /// Retains a capture-index metadata inspection failure.
    pub fn metadata(error: signal_capture::Error) -> Self {
        Self::Metadata(Arc::new(error))
    }

    /// Retains a capture-index opening or construction failure.
    pub fn index(error: signal_capture::Error) -> Self {
        Self::Index(Arc::new(error))
    }
}

impl PartialEq for SourcePreparationError {
    fn eq(&self, other: &Self) -> bool {
        // Preparation is polled. Equivalent capture diagnostics deduplicate while each error
        // value continues to retain the concrete signal-capture source.
        match (self, other) {
            (Self::Discovery(left), Self::Discovery(right)) => left == right,
            (Self::Metadata(left), Self::Metadata(right))
            | (Self::Index(left), Self::Index(right)) => left.to_string() == right.to_string(),
            (Self::Cancelled, Self::Cancelled) => true,
            (Self::Executor(left), Self::Executor(right)) => left == right,
            (Self::WorkerClient(left), Self::WorkerClient(right)) => left == right,
            (Self::Worker(left), Self::Worker(right)) => left == right,
            (Self::WorkerProtocol(left), Self::WorkerProtocol(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for SourcePreparationError {}

#[cfg(test)]
mod source_preparation_contract_tests {
    use std::error::Error as _;

    use platform_runtime::WorkExecutorError;
    use signal_capture::CaptureWorkerMessageKind;

    use super::{SourcePreparationError, SourcePreparationProtocolError};

    #[test]
    fn infrastructure_failures_retain_their_typed_sources() {
        let executor = SourcePreparationError::Executor(WorkExecutorError::QueueFull);
        assert!(executor.source().unwrap().is::<WorkExecutorError>());

        let protocol = SourcePreparationError::WorkerProtocol(
            SourcePreparationProtocolError::UnexpectedResponse {
                received: CaptureWorkerMessageKind::Replay,
            },
        );
        assert!(
            protocol
                .source()
                .unwrap()
                .is::<SourcePreparationProtocolError>()
        );
    }
}

/// Prepared capture data ready for the host viewer and graph runtime.
pub struct PreparedCapture {
    /// Stable source identity associated with the prepared data.
    pub identity: String,
    /// Viewer-channel indexes selected for presentation.
    pub visible_channels: Vec<usize>,
    /// Indexed, in-memory, or metadata-only capture representation.
    pub data: PreparedCaptureData,
}

/// Concrete capture representation produced by source preparation.
pub enum PreparedCaptureData {
    /// Waveform index retained for deferred, random-access viewing.
    Indexed(Box<dyn CaptureIndex + Send>),
    /// Finite waveform data available without an external index.
    InMemory {
        /// Signals and transitions to render.
        signals: Vec<CapturePresentationSignal>,
        /// Capture duration in microseconds.
        duration_us: f64,
    },
    /// Channel names when the source has no waveform data to render.
    Channels(
        /// `(viewer_channel_index, display_name)` entries from the source.
        Vec<(usize, String)>,
    ),
}

/// Presentation information available while a finite capture index is built.
///
/// This deliberately contains only immutable source metadata and build
/// progress. Index ownership remains with source preparation until `Ready`.
pub struct PreparingCapture {
    /// Stable source identity associated with this preparation generation.
    pub identity: String,
    /// Viewer-channel indexes selected for presentation.
    pub visible_channels: Vec<usize>,
    /// Immutable source metadata available before index completion.
    pub metadata: Option<CaptureMetadata>,
    /// Latest progress reported by the index-building operation.
    pub progress: Option<CaptureIndexBuildProgress>,
}

/// Change published by source preparation to the application host.
pub enum SourcePreparationUpdate {
    /// No new state is available.
    Unchanged,
    /// The current prepared capture was removed.
    Cleared,
    /// Finite-source preparation is in progress.
    Preparing(PreparingCapture),
    /// Preparation completed successfully.
    Ready(PreparedCapture),
    /// Preparation failed with a classified owner-defined cause.
    Failed(SourcePreparationError),
}

/// Current phase of finite-source preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePreparationStatus {
    /// No source has supplied preparation state.
    Empty,
    /// Source data or its index is still being prepared.
    Preparing,
    /// Prepared capture data is available.
    Ready,
    /// The latest preparation attempt failed.
    Failed(SourcePreparationError),
}

/// Observable state of the current finite-source preparation generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePreparationSnapshot {
    /// Monotonically increasing preparation generation identifier.
    pub generation: u64,
    /// Current lifecycle phase for this generation.
    pub status: SourcePreparationStatus,
    /// Latest index-build progress, when the source reports it.
    pub progress: Option<CaptureIndexBuildProgress>,
}
