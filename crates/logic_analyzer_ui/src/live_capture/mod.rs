//! Application-level coordination for immediate live capture.

#[cfg(test)]
mod architecture_tests;
mod coordinator;
mod implementation;
#[cfg(test)]
mod test_acquisition_tests;

pub(crate) use coordinator::CaptureCoordinator;
pub(crate) use implementation::{
    CaptureAnalysisAttachment, CaptureAvailability, CaptureCoordinatorContract,
    CaptureFeatureDiscovery, CaptureReplayAttachment, ConfigurationEpochResolution,
    capture_availability,
};

pub(crate) use crate::capture_export_service::CaptureExportFormat as CaptureRawExportFormat;
