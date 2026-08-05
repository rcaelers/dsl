//! UI-independent generic signal, capture, indexing, derived-data, and acquisition contracts.
//!
//! Generic typed-stream execution is owned by `signal_runtime`; this crate consumes its contracts
//! directly and does not re-export them. Concrete devices, formats, decoders, nodes, and sinks live
//! in `logic_analyzer_processing`.

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod wasm_store_tests;

mod advanced_trigger;
pub mod capture;
mod capture_index_kernel;
mod capture_policy;
mod crc32c;
mod derived_data_collector;
mod derived_index;
pub mod derived_word_store;
mod edge_query;
mod errors;
mod events;
pub mod live_capture;
pub mod live_capture_store;
pub mod logic_analyzer;
mod payload;
mod protocol_packet_payload;
mod recorded_edge_query;
mod sample;
mod sampling_points;
mod time_source;
pub mod waveform_index;
mod worker_kernels;

pub use advanced_trigger::{
    RegisteredTriggerPredicateSchema, TRIGGER_PROGRAM_FORMAT_VERSION, TriggerChoice, TriggerCount,
    TriggerCountCapabilities, TriggerCountMode, TriggerEditorSchema, TriggerIdentifier,
    TriggerLogicOperator, TriggerOperandKind, TriggerOperandSchema, TriggerOperandValue,
    TriggerPredicate, TriggerProgram, TriggerProgramEditError, TriggerProgramForm, TriggerStage,
    TriggerValidationCode, TriggerValidationDiagnostic, TriggerValidationErrors,
    ValidatedTriggerProgram,
};
pub use capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProfile, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureIndexOpenStep,
    CaptureIndexOpenTask, CaptureIndexPreparationRequest, CaptureIndexProxy, CaptureIndexQuery,
    CaptureIndexQueryExecutor, CaptureIndexQueryUpdate, CaptureMetadata, CaptureSampledChannel,
    CaptureSampledWindow, CaptureSampledWindowPoll, CaptureSource, CaptureTransition,
    CaptureWaveformSegment, CaptureWorkerClient, CaptureWorkerIndexQueryExecutor,
    CaptureWorkerMessage, CaptureWorkerOperationRegistry, CaptureWorkerPreparedIndex,
    CaptureWorkerReplayBlock, CaptureWorkerReplayRequest, CaptureWorkerReplaySource,
    CaptureWorkerRequest, CaptureWorkerRuntime, IndexedCapturePresentation,
    decode_capture_worker_messages, encode_capture_worker_messages, packed_bit,
};
pub use capture_policy::{
    CaptureFraction, CapturePolicy, CapturePolicyCapabilities, CapturePolicyContext,
    CapturePolicyError, CaptureRetentionPin, CaptureRetentionTracker, CaptureSessionPlan,
    CaptureStartMode, CompletionPolicy, CompletionPolicyKind, EffectiveCapturePolicy,
    RecordingStart, RetentionPolicy, RetentionPolicyKind, TriggerPlacement,
    TriggerPlacementCapability, TriggerTimeout, TriggerTimeoutAction,
};
pub use derived_data_collector::{
    CollectedWordLaneOptions, CollectedWordLaneQuery, DEFAULT_DERIVED_DATA_MAX_ENTRIES,
    DerivedDataCollector, DerivedDataCollectorMetrics, DerivedDataCollectorMetricsSnapshot,
    DerivedDataRetention, DerivedLanes, DigitalLaneSnapshot, IndexedAnnotationLane,
    NumberLaneSnapshot, OpaqueCollectedLane, TextLaneSnapshot, TriggerLaneSnapshot,
    WordLaneSnapshot, built_in_word_lane_ingestor, digital_payload_adapter, number_payload_adapter,
    text_payload_adapter, trigger_payload_adapter, word_payload_adapter,
};
pub use derived_index::{AppendOnlyMipmap, ChunkedMipmap, LaneFold, MipmapRecord};
pub use derived_word_store::{
    AnnotationQuery, BlockCodecConfig, DecodedBlockCacheStats, IndexedAnnotationStore,
    IndexedAnnotationWriter, LiveStoreConfig, PersistentStoreConfig, StoreStatus,
    WordPresenceBucket, cleanup_cache, clear_cache, clear_cache_entry,
    configure_decoded_block_cache, decoded_block_cache_stats, reset_decoded_block_cache_stats,
};
pub use edge_query::{
    EdgeQuery, EdgeQueryInputPortExt, EdgeQueryProcessNodeExt, edge_query_capability,
    edge_query_from_capability, edge_query_protocol,
};
pub use errors::{Error, Result};
pub use events::{
    Annotation, MAX_ANNOTATION_NS, NumberSample, ProtocolPacket, ProtocolValue, TextSample,
    TimelineMarker, Trigger, Word, WordPayload, instantaneous_word_end_ns,
};
pub use live_capture::{
    AcquisitionContext, AcquisitionError, AcquisitionOutcome, AcquisitionResult,
    CAPTURE_CHUNK_FORMAT_VERSION, CaptureAcquisitionPhase, CaptureAnalysisChannel,
    CaptureAnalysisSource, CaptureBufferLease, CaptureBufferPool, CaptureBufferPoolError,
    CaptureBufferPoolMetrics, CaptureBytes, CaptureChannelId, CaptureChunk, CaptureChunkError,
    CaptureChunkPayload, CaptureChunkWriter, CaptureCommandCapabilities, CaptureCompletion,
    CaptureDataDelivery, CaptureEvent, CaptureEventPublishError, CaptureEventPublisher,
    CaptureEventQueuePublisher, CaptureEventQueueReader, CaptureFailure, CaptureFailureKind,
    CaptureHealth, CaptureProgress, CaptureProviderCapabilities, CaptureQueueConfigError,
    CaptureQueueLimits, CaptureQueueReader, CaptureQueueReceiveError, CaptureQueueWriter,
    CaptureSessionId, CaptureSessionState, CaptureSettingCombination, CaptureStatus,
    CaptureWriteError, ConfiguredAcquisition, PreparedAcquisition, SimpleTriggerCondition,
    bounded_capture_event_queue, bounded_capture_queue,
};
pub use live_capture_store::*;
pub use payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
    PayloadDescriptor, PayloadRegistrationError, PayloadRegistry,
};
pub use protocol_packet_payload::{ProtocolPacketLaneSnapshot, protocol_packet_payload_adapter};
pub use recorded_edge_query::RecordedEdgeQuery;
pub use sample::{Sample, SampleBlock};
pub use sampling_points::{
    PackedSamplingPoint, PackedSamplingPointBatch, SamplingPoint, SamplingPointProvider,
    SamplingPointStore,
};
pub use time_source::{SystemUnixTimeSource, UnixTimeSource};
pub use waveform_index::{
    CaptureIndexProgress, GrowingCaptureIndex, GrowingCaptureIndexWorker, IndexSampler,
    exact_window_sample_limit,
};
pub use worker_kernels::portable_worker_kernels;
