//! UI-owned asynchronous capture-export boundary.

mod implementation;
#[cfg(test)]
mod test_implementation_tests;

pub use implementation::unavailable_capture_export_service;
pub use logic_analyzer_capture_export::{
    CaptureExportCompletion, CaptureExportDescriptor, CaptureExportFormat, CaptureExportService,
    CaptureExportStatus,
};
#[cfg(test)]
pub(crate) use test_implementation_tests::{
    ScriptedCaptureExportControl, scripted_capture_export_service,
};
