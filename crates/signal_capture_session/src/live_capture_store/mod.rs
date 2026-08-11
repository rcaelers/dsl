//! # `signal_capture_session::live_capture_store`
//!
//! ## Responsibility
//!
//! This module owns the authoritative append-only storage and replay contracts for a live capture
//! session, including committed-prefix visibility and finalized-session access.
//!
//! ## Boundaries
//!
//! It uses generic artifact storage and contains no device protocol, path policy, UI state, or derived
//! processing policy. Capture coordination decides session replacement and presentation attachment.

//! Platform-neutral authoritative live-capture storage.
//!
//! It owns recording, cursor, finalization, recovery, and session-repository
//! contracts. Physical backing is injected through generic artifact capabilities.

mod artifact_store;
mod records;
mod session_repository;

pub use artifact_store::{
    CaptureCursor, CaptureRandomReader, CaptureStore, CaptureStoreConfig, CaptureStoreWriter,
    FinalizedCapture,
};
pub use records::{
    CaptureCursorItem, CaptureReclamationReport, CaptureRecordingGate, CaptureRecoveryReport,
    CaptureSessionMetadata, CaptureSessionOutcome, CaptureStoreCursor, CaptureStoreDescriptor,
    CaptureStoreError, CaptureStoreManifest, CaptureStoreResult, CaptureStoreSnapshot,
    CaptureTimelineMetadata, RecordingCaptureCursor,
};
pub use session_repository::{
    CaptureSessionCleanupPlan, CaptureSessionPin, CaptureSessionRepository,
    CaptureSessionRepositoryConfig, CaptureSessionSummary,
};
