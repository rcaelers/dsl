//! Generic derived-signal payloads, collection, indexing, sampling, and encoded storage.
//!
//! This crate owns presentation-neutral retained outputs. It does not own immutable capture,
//! acquisition sessions, concrete protocols or nodes, viewers, graph documents, or host adapters.

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod wasm_store_tests;

mod derived_data_collector;
mod derived_index;
pub mod derived_word_store;
mod events;
mod payload;
mod payload_ingestor_construction_error;
mod sampling_points;
mod worker_kernels;

pub use derived_data_collector::{
    CollectedWordLaneOptions, CollectedWordLaneQuery, DEFAULT_DERIVED_DATA_MAX_ENTRIES,
    DerivedDataCollector, DerivedDataCollectorMetrics, DerivedDataCollectorMetricsSnapshot,
    DerivedDataRetention, DerivedLanes, DigitalLaneSnapshot, IndexedAnnotationLane,
    NumberLaneSnapshot, OpaqueCollectedLane, TextLaneSnapshot, TimestampEventLaneSnapshot,
    WordLaneSnapshot, built_in_word_lane_ingestor, digital_payload_adapter, number_payload_adapter,
    text_payload_adapter, timestamp_event_payload_adapter, word_payload_adapter,
};
pub use derived_index::{AppendOnlyMipmap, ChunkedMipmap, LaneFold, MipmapRecord};
pub use derived_word_store::{
    AnnotationQuery, BlockCodecConfig, DecodedBlockCacheHandle, DecodedBlockCacheStats,
    IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig, PersistentStoreConfig,
    StoreStatus, WordPresenceBucket, cleanup_cache, clear_cache, clear_cache_entry,
};
pub use events::{
    Annotation, MAX_ANNOTATION_NS, NumberSample, TextSample, TimelineMarker, TimestampEvent, Word,
    WordPayload, instantaneous_word_end_ns,
};
pub use payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
    PayloadDescriptor, PayloadRegistrationError, PayloadRegistry,
};
pub use payload_ingestor_construction_error::PayloadIngestorConstructionError;
pub use sampling_points::{
    PackedSamplingPoint, PackedSamplingPointBatch, SamplingPoint, SamplingPointProvider,
    SamplingPointStore,
};
pub use worker_kernels::portable_worker_kernels;
