//! UI-independent signal-processing runtime and capture infrastructure.
//!
//! This library provides a memory-efficient node runtime for processing captured and live
//! signals. Concrete logic-analyzer sources, processors, and sinks live in
//! `logic-analyzer-processing`.
//!
//! # Architecture
//!
//! - **Capture contracts**: Generic interfaces for sampled and indexed signals
//! - **Streaming Nodes**: Thread-per-node execution with crossbeam channels
//! - **Scheduler**: Manages node lifecycle and parallel execution
//! - **Derived data**: Generic viewer-lane storage and queries

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod wasm_store_tests;

mod advanced_trigger;
mod app_manager;
pub mod capture;
mod capture_index_kernel;
mod capture_policy;
mod cooperative_manager;
mod crc32c;
mod derived_data_collector;
mod derived_index;
pub mod derived_word_store;
mod edge_query;
mod errors;
mod events;
mod graph;
pub mod live_capture;
pub mod live_capture_store;
pub mod logic_analyzer;
mod manager;
mod node;
mod payload;
mod pipeline;
mod ports;
mod protocol;
mod protocol_packet_payload;
mod receiver;
mod recorded_edge_query;
mod sample;
mod sample_kind;
mod sampling_points;
mod scheduler;
mod sender;
mod storage;
mod time_source;
mod type_registry;
mod watchdog;
pub mod waveform_index;
mod work_executor;
mod worker_kernels;
mod worker_operation_queue;

pub use advanced_trigger::{
    RegisteredTriggerPredicateSchema, TRIGGER_PROGRAM_FORMAT_VERSION, TriggerChoice, TriggerCount,
    TriggerCountCapabilities, TriggerCountMode, TriggerEditorSchema, TriggerIdentifier,
    TriggerLogicOperator, TriggerOperandKind, TriggerOperandSchema, TriggerOperandValue,
    TriggerPredicate, TriggerProgram, TriggerProgramEditError, TriggerProgramForm, TriggerStage,
    TriggerValidationCode, TriggerValidationDiagnostic, TriggerValidationErrors,
    ValidatedTriggerProgram,
};
pub use app_manager::{
    AppManager, AppManagerBackend, AppManagerFactory, CooperativeAppManagerBackend,
    CooperativeAppManagerFactory,
};
pub use capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProgress, CaptureIndexFactory, CaptureMetadata, CaptureSampledChannel,
    CaptureSampledWindow, CaptureSource, CaptureTransition, CaptureWaveformSegment,
    IndexedCapturePresentation, packed_bit,
};
pub use capture_policy::{
    CaptureFraction, CapturePolicy, CapturePolicyCapabilities, CapturePolicyContext,
    CapturePolicyError, CaptureRetentionPin, CaptureRetentionTracker, CaptureSessionPlan,
    CaptureStartMode, CompletionPolicy, CompletionPolicyKind, EffectiveCapturePolicy,
    RecordingStart, RetentionPolicy, RetentionPolicyKind, TriggerPlacement,
    TriggerPlacementCapability, TriggerTimeout, TriggerTimeoutAction,
};
pub use cooperative_manager::CooperativeManager;
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
pub use edge_query::EdgeQuery;
pub use errors::{ConnectionError, Error, PortError, Result, WorkError, WorkResult};
pub use events::{
    Annotation, MAX_ANNOTATION_NS, NumberSample, ProtocolPacket, ProtocolValue, TextSample,
    TimelineMarker, Trigger, Word, WordPayload, instantaneous_word_end_ns,
};
pub use graph::{Connection, GraphBuilder, NodeId};
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
pub use manager::{DisconnectEvent, InputSub, NodeSpec, PipelineManager};
pub use node::{
    ConfigOutcome, ConfigValue, ConfigurationBoundary, ConfigurationScheduler,
    InputProtocolCandidate, NodeCancellation, NodeConfig, ProcessNode, WorkOutcome,
};
pub use payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
    PayloadDescriptor, PayloadRegistrationError, PayloadRegistry,
};
pub use pipeline::Pipeline;
pub use ports::{InputPort, OutputPort, PortDirection, PortSchema, register_type};
pub use protocol::ProtocolKind;
pub use protocol_packet_payload::{ProtocolPacketLaneSnapshot, protocol_packet_payload_adapter};
pub use receiver::{Receiver, ReceiverSelector};
pub use recorded_edge_query::RecordedEdgeQuery;
pub use sample::{Sample, SampleBlock};
pub use sample_kind::SampleKind;
pub(crate) use sample_kind::negotiate as negotiate_sample_kind;
pub use sampling_points::{SamplingPoint, SamplingPointProvider, SamplingPointStore};
pub use scheduler::{Scheduler, StopHandle};
pub use sender::{ChannelMessage, OverflowPolicy, Sender, SharedSenders};
pub use storage::{
    ArtifactByteSource, ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository,
    ByteRange, ByteRegion, ChunkedByteSource, ImmutableByteRegion, MemoryArtifactRepository,
    OwnedByteSource, PreparedByteSource, RandomAccessReader, ReadArtifact, RepositoryCapabilities,
    RepositoryError, SourceCapabilities, SourceIdentity, SourceReadError, WriteArtifact,
    read_artifact_region,
};
pub use time_source::{SystemUnixTimeSource, UnixTimeSource};
pub(crate) use watchdog::OperationGuard;
pub use watchdog::{Watchdog, WatchdogHandle};
pub use waveform_index::{
    CaptureIndexProgress, GrowingCaptureIndex, GrowingCaptureIndexWorker, IndexSampler,
    exact_window_sample_limit,
};
pub use work_executor::{
    CompletedWorkTask, CooperativeWorkerOperationExecutor, InlineWorkExecutor, WorkExecutor,
    WorkExecutorTask, WorkTask, WorkerExecutionCapability, WorkerExecutionMode,
    WorkerKernelRegistry, WorkerMessage, WorkerMessageError, WorkerOperation,
    WorkerOperationExecutor, WorkerRequest,
};
pub use worker_kernels::portable_worker_kernels;
pub use worker_operation_queue::{WorkerHostCommand, WorkerOperationQueue};
