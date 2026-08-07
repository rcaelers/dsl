//! Portable logic-analyzer driver, capture configuration, and runtime-source contracts.
//!
//! This crate owns device-neutral acquisition concepts shared by concrete logic-analyzer drivers
//! and graph sources. Device protocols, transports, capture-session coordination, graph
//! definitions, and presentation remain in their corresponding owners.

#[cfg(test)]
mod architecture_tests;
mod driver;
mod trigger;

pub use driver::{
    CaptureMode, ClockEdge, ClockSource, LogicAnalyzer, LogicAnalyzerError, LogicAnalyzerInfo,
    LogicAnalyzerResult, LogicAnalyzerSource, LogicCaptureConfig, LogicChunk, LogicEncoding,
    LogicEncodingRequest,
};
pub use trigger::{LogicTrigger, LogicTriggerStage, TriggerCondition, TriggerLogic};
