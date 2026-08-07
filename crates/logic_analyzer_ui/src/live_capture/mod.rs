//! Application-level coordination for immediate live capture.
//!
//! The module owns the invariant-preserving composition of one acquisition state machine, its
//! published capture artifacts, and the status projected to application controls. Sibling modules
//! consume the `CaptureCoordinator` and `CaptureCoordinatorContract` facades. The owner may depend
//! on generic capture, graph, runtime, repository, and export-service contracts; it excludes
//! concrete devices and protocols, graph execution policy, widget rendering, and host adapters.

mod acquisition_state;
#[cfg(test)]
mod architecture_tests;
mod coordinator;
mod implementation;
mod status_projection;
mod storage_publication;
#[cfg(test)]
mod test_acquisition_tests;

pub(crate) use coordinator::CaptureCoordinator;
pub(crate) use implementation::{
    CaptureAnalysisAttachment, CaptureAvailability, CaptureCoordinatorContract,
    CaptureFeatureDiscovery, CaptureReplayAttachment, ConfigurationEpochResolution,
    capture_availability,
};

pub(crate) use crate::capture_export_service::CaptureExportFormat as CaptureRawExportFormat;
