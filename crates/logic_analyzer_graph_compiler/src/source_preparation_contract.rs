use logic_analyzer_graph_api::node_support::CapturePresentationSignal;
use signal_processing::{CaptureIndex, CaptureIndexBuildProgress, CaptureMetadata};

pub struct PreparedCapture {
    pub identity: String,
    pub visible_channels: Vec<usize>,
    pub data: PreparedCaptureData,
}

pub enum PreparedCaptureData {
    Indexed(Box<dyn CaptureIndex + Send>),
    InMemory {
        signals: Vec<CapturePresentationSignal>,
        duration_us: f64,
    },
    Channels(Vec<(usize, String)>),
}

/// Presentation information available while a finite capture index is built.
///
/// This deliberately contains only immutable source metadata and build
/// progress. Index ownership remains with source preparation until `Ready`.
pub struct PreparingCapture {
    pub identity: String,
    pub visible_channels: Vec<usize>,
    pub metadata: Option<CaptureMetadata>,
    pub progress: Option<CaptureIndexBuildProgress>,
}

pub enum SourcePreparationUpdate {
    Unchanged,
    Cleared,
    Preparing(PreparingCapture),
    Ready(PreparedCapture),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePreparationStatus {
    Empty,
    Preparing,
    Ready,
    Failed(String),
}

/// Observable state of the current finite-source preparation generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePreparationSnapshot {
    pub generation: u64,
    pub status: SourcePreparationStatus,
    pub progress: Option<CaptureIndexBuildProgress>,
}
