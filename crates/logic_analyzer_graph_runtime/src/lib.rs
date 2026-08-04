//! Materialization and lifecycle services for compiled logic-analyzer graphs.
//!
//! The compiler owns document semantics and produces immutable execution plans. This crate owns
//! the resources whose lifetime begins when a plan is prepared or run: repositories, executors,
//! runtime managers, source preparation, and cache maintenance. Processing-plan contracts live in
//! `logic-analyzer-graph-plan`; worker composition lives above this crate.

mod runtime;

#[cfg(test)]
mod architecture_tests;

pub use runtime::*;
