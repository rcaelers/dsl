//! # Capture contracts
//!
//! ## Responsibility
//!
//! This module owns generic immutable capture-source, index, query, and worker-operation contracts.
//! It describes sampled data and prepared random access without naming a file format, device, path, or
//! viewer.
//!
//! ## Boundaries
//!
//! Concrete parsers and source nodes implement these contracts above `signal_capture`. Artifact
//! backing is supplied through generic storage contracts; waveform summary construction belongs to
//! `waveform_index`.

//! Generic immutable capture contracts and packed capture data.
//!
//! The module owns source, index, query, and worker-operation contracts for sampled
//! data and prepared random access. Concrete parsers and source nodes implement
//! these contracts above `signal_capture`; formats, devices, paths, and viewer
//! policy are intentionally absent.

mod host_protocol;
mod implementation;
mod preparation;
mod query;
mod worker_client;
mod worker_errors;
mod worker_operation_errors;
mod worker_replay_source;
mod worker_runtime;

pub use host_protocol::{
    CaptureWorkerMessage, CaptureWorkerReplayBlock, CaptureWorkerReplayRequest,
    CaptureWorkerRequest, decode_capture_worker_messages, decode_capture_worker_request,
    encode_capture_worker_messages, encode_capture_worker_request,
};
pub use implementation::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProfile, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureIndexOpenStep,
    CaptureIndexOpenTask, CaptureMetadata, CaptureSampledChannel, CaptureSampledWindow,
    CaptureSampledWindowPoll, CaptureSource, CaptureTransition, CaptureWaveformSegment,
    IndexedCapturePresentation, packed_bit,
};
pub use preparation::CaptureIndexPreparationRequest;
pub use query::{
    CaptureIndexProxy, CaptureIndexQuery, CaptureIndexQueryError, CaptureIndexQueryExecutor,
    CaptureIndexQueryUpdate,
};
pub use worker_client::{CaptureWorkerClient, CaptureWorkerIndexQueryExecutor};
pub use worker_errors::{
    CaptureWorkerClientError, CaptureWorkerCodecError, CaptureWorkerFailure, CaptureWorkerFrame,
    CaptureWorkerMessageKind, CaptureWorkerRequestKind, CaptureWorkerTransportFailure,
};
pub use worker_operation_errors::{
    CaptureWorkerOperationPreparationError, CaptureWorkerOperationRegistrationError,
};
pub use worker_replay_source::CaptureWorkerReplaySource;
pub use worker_runtime::{
    CaptureWorkerOperationRegistry, CaptureWorkerPreparedIndex, CaptureWorkerRuntime,
};
