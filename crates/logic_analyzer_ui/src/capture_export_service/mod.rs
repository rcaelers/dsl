//! UI-owned asynchronous capture-export boundary.

#[cfg(test)]
mod architecture_tests;
mod contract;
mod implementation;
#[cfg(test)]
mod test_implementation_tests;

pub use contract::{
    CaptureExportCompletion, CaptureExportDescriptor, CaptureExportFormat, CaptureExportService,
    CaptureExportStatus,
};
pub use implementation::unavailable_capture_export_service;
#[cfg(test)]
pub(crate) use test_implementation_tests::{
    ScriptedCaptureExportControl, scripted_capture_export_service,
};
