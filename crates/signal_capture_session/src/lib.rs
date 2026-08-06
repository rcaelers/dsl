//! UI-independent generic capture-session contracts.
//!
//! Immutable capture and finite indexing are owned by `signal_capture`, retained derived outputs by
//! `signal_derived`, and typed-stream execution by `signal_runtime`. This crate consumes those
//! contracts directly and does not re-export them. Concrete devices, formats, decoders, nodes, and
//! sinks live in positive-responsibility domain crates above this generic owner.

mod advanced_trigger;
#[cfg(test)]
mod architecture_tests;
mod capture_policy;
mod capture_source_metadata;
mod growing_capture_index;
pub mod live_capture;
pub mod live_capture_store;
pub mod logic_analyzer;

pub use advanced_trigger::{
    RegisteredTriggerPredicateSchema, TRIGGER_PROGRAM_FORMAT_VERSION, TriggerChoice, TriggerCount,
    TriggerCountCapabilities, TriggerCountMode, TriggerEditorSchema, TriggerIdentifier,
    TriggerLogicOperator, TriggerOperandKind, TriggerOperandSchema, TriggerOperandValue,
    TriggerPredicate, TriggerProgram, TriggerProgramEditError, TriggerProgramForm, TriggerStage,
    TriggerValidationCode, TriggerValidationDiagnostic, TriggerValidationErrors,
    ValidatedTriggerProgram,
};
pub use capture_policy::{
    CaptureFraction, CapturePolicy, CapturePolicyCapabilities, CapturePolicyContext,
    CapturePolicyError, CaptureRetentionPin, CaptureRetentionTracker, CaptureSessionPlan,
    CaptureStartMode, CompletionPolicy, CompletionPolicyKind, EffectiveCapturePolicy,
    RecordingStart, RetentionPolicy, RetentionPolicyKind, TriggerPlacement,
    TriggerPlacementCapability, TriggerTimeout, TriggerTimeoutAction,
};
pub use capture_source_metadata::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, CaptureSourceRuntimeCapabilities, CaptureSourceSignal,
};
pub use growing_capture_index::{GrowingCaptureIndex, GrowingCaptureIndexWorker};
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
