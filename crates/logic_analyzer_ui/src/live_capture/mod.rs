//! Application-level coordination for immediate live capture.

#[cfg(test)]
mod architecture_tests;
mod implementation;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod test_acquisition_tests;

#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod platform;
#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod platform;

pub(crate) use implementation::{
    CaptureAnalysisAttachment, CaptureAvailability, CaptureCoordinatorContract,
    CaptureFeatureDiscovery, CaptureReplayAttachment, ConfigurationEpochResolution,
    capture_availability,
};
#[cfg(target_arch = "wasm32")]
pub(crate) use platform::CaptureCoordinator;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use platform::CaptureCoordinator;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use crate::capture_export_service::CaptureExportFormat as CaptureRawExportFormat;
