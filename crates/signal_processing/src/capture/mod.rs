//! Generic immutable capture contracts and packed capture data.

mod host_protocol;
mod implementation;
mod preparation;
mod query;
mod worker_client;
mod worker_replay_source;
mod worker_runtime;

pub use host_protocol::{
    CaptureWorkerMessage, CaptureWorkerReplayBlock, CaptureWorkerReplayRequest,
    CaptureWorkerRequest, decode_capture_worker_messages, encode_capture_worker_messages,
};
pub use implementation::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProgress, CaptureIndexFactory, CaptureIndexOpenStep, CaptureIndexOpenTask,
    CaptureMetadata, CaptureSampledChannel, CaptureSampledWindow, CaptureSampledWindowPoll,
    CaptureSource, CaptureTransition, CaptureWaveformSegment, IndexedCapturePresentation,
    packed_bit,
};
pub use preparation::CaptureIndexPreparationRequest;
pub use query::{
    CaptureIndexProxy, CaptureIndexQuery, CaptureIndexQueryExecutor, CaptureIndexQueryUpdate,
};
pub use worker_client::{CaptureWorkerClient, CaptureWorkerIndexQueryExecutor};
pub use worker_replay_source::CaptureWorkerReplaySource;
pub use worker_runtime::{
    CaptureWorkerOperationRegistry, CaptureWorkerPreparedIndex, CaptureWorkerRuntime,
};
