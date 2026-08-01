//! Platform-neutral authoritative live-capture storage.

mod artifact_store;
mod implementation;
mod session_repository;

pub use artifact_store::{
    CaptureCursor, CaptureRandomReader, CaptureStore, CaptureStoreConfig, CaptureStoreWriter,
    FinalizedCapture,
};
pub use implementation::{
    CaptureCursorItem, CaptureReclamationReport, CaptureRecordingGate, CaptureRecoveryReport,
    CaptureSessionMetadata, CaptureSessionOutcome, CaptureStoreCursor, CaptureStoreDescriptor,
    CaptureStoreError, CaptureStoreManifest, CaptureStoreResult, CaptureStoreSnapshot,
    CaptureTimelineMetadata, RecordingCaptureCursor,
};
pub use session_repository::{
    CaptureSessionCleanupPlan, CaptureSessionPin, CaptureSessionRepository,
    CaptureSessionRepositoryConfig, CaptureSessionSummary,
};
