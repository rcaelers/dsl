//! UI-owned asynchronous capture-export boundary.

#[cfg(test)]
mod architecture_tests;
mod contract;
#[cfg(all(not(target_arch = "wasm32"), feature = "native-host"))]
#[path = "native.rs"]
mod implementation;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "native-host")))]
#[path = "unavailable_native.rs"]
mod implementation;
#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod implementation;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod test_implementation_tests;

pub use contract::{
    CaptureExportCompletion, CaptureExportDescriptor, CaptureExportFormat, CaptureExportService,
    CaptureExportStatus,
};
pub(crate) use implementation::standard_capture_export_service;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use test_implementation_tests::{
    ScriptedCaptureExportControl, scripted_capture_export_service,
};
