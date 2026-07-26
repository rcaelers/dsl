//! DSLogic U3Pro16 source node, USB driver, and acquisition profiles.

#[cfg(feature = "developer-tools")]
mod benchmark;
mod buffered;
mod capture;
mod common;
mod implementation;
mod source;
mod streaming;

#[cfg(feature = "developer-tools")]
pub use benchmark::run_streaming_benchmark;
pub use capture::DsLogicU3Pro16Capture;
pub use signal_processing::logic_analyzer::{
    CaptureMode, ClockEdge, ClockSource, LogicCaptureConfig, LogicEncodingRequest, LogicTrigger,
    LogicTriggerStage, TriggerCondition, TriggerLogic,
};
pub use source::DsLogicU3Pro16Source;
