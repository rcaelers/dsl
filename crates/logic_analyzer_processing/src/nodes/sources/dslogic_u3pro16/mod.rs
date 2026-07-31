//! DSLogic U3Pro16 source node, USB driver, and acquisition profiles.

#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
mod benchmark;
#[cfg(not(target_arch = "wasm32"))]
mod buffered;
#[cfg(not(target_arch = "wasm32"))]
mod capture;
#[cfg(not(target_arch = "wasm32"))]
mod common;
mod facade;
#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
mod hardware_validation;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
mod platform;
#[cfg(not(target_arch = "wasm32"))]
mod source;
#[cfg(not(target_arch = "wasm32"))]
mod streaming;

#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
pub use benchmark::run_streaming_benchmark;
#[cfg(not(target_arch = "wasm32"))]
pub use capture::DsLogicU3Pro16Capture;
pub use facade::create_source;
#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
pub use hardware_validation::{validate_capture_hardware, validate_fpga_hardware};
pub use signal_processing::logic_analyzer::{
    CaptureMode, ClockEdge, ClockSource, LogicCaptureConfig, LogicEncodingRequest, LogicTrigger,
    LogicTriggerStage, TriggerCondition, TriggerLogic,
};
#[cfg(not(target_arch = "wasm32"))]
pub use source::DsLogicU3Pro16Source;
