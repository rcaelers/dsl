//! UI-owned asynchronous capture-export boundary.

#[cfg(test)]
mod test_implementation_tests;
mod unavailable;

pub use logic_analyzer_capture_export::{
    CaptureExportCompletion, CaptureExportDescriptor, CaptureExportFormat, CaptureExportService,
    CaptureExportServiceError, CaptureExportStatus,
};
#[cfg(test)]
pub(crate) use test_implementation_tests::{
    ScriptedCaptureExportControl, scripted_capture_export_service,
};
pub use unavailable::unavailable_capture_export_service;
