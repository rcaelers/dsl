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
#[cfg(not(target_arch = "wasm32"))]
#[path = "platform_contract_native.rs"]
mod platform_contract;
#[cfg(target_arch = "wasm32")]
#[path = "platform_contract_wasm.rs"]
mod platform_contract;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod test_implementation_tests;

pub(crate) use contract::{CaptureExportCompletion, CaptureExportService, CaptureExportStatus};
pub(crate) use implementation::standard_capture_export_service;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use platform_contract::CaptureExportFormat;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use test_implementation_tests::{
    ScriptedCaptureExportControl, scripted_capture_export_service,
};
