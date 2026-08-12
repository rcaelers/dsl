//! Generic immutable signal capture, random-access queries, and finite waveform indexing.
//!
//! This crate owns capture payloads and index contracts without knowing concrete formats, devices,
//! graph nodes, viewers, acquisition sessions, or derived-data stores.

#[cfg(test)]
mod architecture_tests;

mod capture;
mod capture_index_kernel;
mod channel_identity;
mod edge_query;
mod errors;
mod recorded_edge_query;
mod sample;
mod waveform_index;

pub use capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProfile, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureIndexOpenStep,
    CaptureIndexOpenTask, CaptureIndexPreparationRequest, CaptureIndexProxy, CaptureIndexQuery,
    CaptureIndexQueryError, CaptureIndexQueryExecutor, CaptureIndexQueryUpdate, CaptureMetadata,
    CaptureReaderPurpose, CaptureSampledChannel, CaptureSampledWindow, CaptureSampledWindowPoll,
    CaptureSource, CaptureTransition, CaptureWaveformSegment, CaptureWorkerClient,
    CaptureWorkerClientError, CaptureWorkerCodecError, CaptureWorkerFailure, CaptureWorkerFrame,
    CaptureWorkerIndexQueryExecutor, CaptureWorkerMessage, CaptureWorkerMessageKind,
    CaptureWorkerOperationPreparationError, CaptureWorkerOperationRegistrationError,
    CaptureWorkerOperationRegistry, CaptureWorkerPreparedIndex, CaptureWorkerReplayBlock,
    CaptureWorkerReplayRequest, CaptureWorkerReplaySource, CaptureWorkerRequest,
    CaptureWorkerRequestKind, CaptureWorkerRuntime, CaptureWorkerTransportFailure,
    IndexedCapturePresentation, decode_capture_worker_messages, decode_capture_worker_request,
    encode_capture_worker_messages, encode_capture_worker_request, packed_bit,
};
pub use capture_index_kernel::register_capture_worker_kernel;
pub use channel_identity::CaptureChannelId;
pub use edge_query::{
    EdgeQuery, EdgeQueryInputPortExt, EdgeQueryProcessNodeExt, edge_query_capability,
    edge_query_from_capability, edge_query_protocol,
};
pub use errors::{Error, Result};
pub use recorded_edge_query::RecordedEdgeQuery;
pub use sample::{Sample, SampleBlock};
pub use waveform_index::{
    CaptureIndexProgress, IndexSampler, WaveformSummary, WaveformSummaryGrid,
    exact_window_sample_limit, sample_waveform_summary_channel, select_waveform_summary_resolution,
};
