use logic_analyzer_graph_api::node_support::CapturePresentationSignal;
use signal_processing::{CaptureIndex, CaptureIndexBuildProgress, CaptureMetadata};

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
    /// Preparation failed with a user-presentable error.
    Failed(String),
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
    Failed(String),
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
