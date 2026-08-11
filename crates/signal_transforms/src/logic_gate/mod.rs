//! # `logic_gate`
//!
//! ## Responsibility
//!
//! This module owns boolean level-stream logic operations over its configured inputs.
//!
//! ## Boundaries
//!
//! It does not own variadic socket editing, node titles, or input visualization. Those are graph-node
//! and editor concerns.

//! Boolean logic-gate processing node.
//!
//! The implementation transforms generic digital streams. Node definitions and
//! socket presentation remain with the corresponding graph-node feature.

mod edge_query;
mod gate;

pub use gate::{GateOp, LogicGate};
