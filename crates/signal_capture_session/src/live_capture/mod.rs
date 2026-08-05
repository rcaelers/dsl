//! # `signal_capture_session::live_capture`
//!
//! ## Responsibility
//!
//! This module owns driver-neutral acquisition configuration, commands, events, progress, queueing,
//! and terminal outcomes for a live capture session.
//!
//! ## Boundaries
//!
//! It does not implement a device transport, create a graph run, persist an application document, or
//! present controls. Concrete sources implement its provider contracts and the UI coordinates sessions.

//! Provider-neutral live-capture contracts, events, queues, and buffers.
//!
//! Providers implement configured and prepared acquisition contracts while this
//! module owns portable lifecycle, delivery, trigger, and bounded-buffer vocabulary.
//! Device transport, UI workflow, and target selection belong to other owners.

mod acquisition;
mod analysis;
mod implementation;

pub use acquisition::{
    AcquisitionContext, AcquisitionError, AcquisitionOutcome, AcquisitionResult,
    ConfiguredAcquisition, PreparedAcquisition,
};
pub use analysis::{CaptureAnalysisChannel, CaptureAnalysisSource};
pub use implementation::{
    CAPTURE_CHUNK_FORMAT_VERSION, CaptureAcquisitionPhase, CaptureBufferLease, CaptureBufferPool,
    CaptureBufferPoolError, CaptureBufferPoolMetrics, CaptureBytes, CaptureChannelId, CaptureChunk,
    CaptureChunkError, CaptureChunkPayload, CaptureChunkWriter, CaptureCommandCapabilities,
    CaptureCompletion, CaptureDataDelivery, CaptureEvent, CaptureEventPublishError,
    CaptureEventPublisher, CaptureEventQueuePublisher, CaptureEventQueueReader, CaptureFailure,
    CaptureFailureKind, CaptureHealth, CaptureProgress, CaptureProviderCapabilities,
    CaptureQueueConfigError, CaptureQueueLimits, CaptureQueueReader, CaptureQueueReceiveError,
    CaptureQueueWriter, CaptureSessionId, CaptureSessionState, CaptureSettingCombination,
    CaptureStatus, CaptureWriteError, SimpleTriggerCondition, bounded_capture_event_queue,
    bounded_capture_queue,
};
