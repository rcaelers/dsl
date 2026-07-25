use logic_analyzer_graph_api::node_support::CapturePresentationSignal;
use signal_processing::CaptureIndex;

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

pub enum SourcePreparationUpdate {
    Unchanged,
    Cleared,
    Preparing,
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
