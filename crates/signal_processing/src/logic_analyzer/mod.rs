//! # `signal_processing::logic_analyzer`
//!
//! ## Responsibility
//!
//! This module owns driver-neutral logic-analyser source, trigger, and capture-configuration
//! contracts consumed by concrete device processing nodes.
//!
//! ## Boundaries
//!
//! It does not select a USB implementation, define a concrete device model, render configuration, or
//! persist graph-node state. Device-specific validation and transport remain in processing and platform
//! owners.

//! Shared logic-analyzer driver and source-adaptation support.
//!
//! This namespace defines driver-neutral capture, trigger, and processing-source
//! contracts consumed by concrete device nodes. USB/device transport and graph/UI
//! behavior remain outside the generic runtime.

mod implementation;
mod trigger;

pub use implementation::{
    CaptureMode, ClockEdge, ClockSource, LogicAnalyzer, LogicAnalyzerError, LogicAnalyzerInfo,
    LogicAnalyzerResult, LogicAnalyzerSource, LogicCaptureConfig, LogicChunk, LogicEncoding,
    LogicEncodingRequest,
};
pub use trigger::{LogicTrigger, LogicTriggerStage, TriggerCondition, TriggerLogic};
