//! Inventory-backed registration and catalog assembly for graph features and payloads.
//!
//! Graph plugins submit descriptors owned here while implementing capability contracts from
//! `logic_analyzer_graph_capabilities`. Compiler, runtime, and UI consumers read the same deterministically
//! validated inventory without owning plugin discovery policy.

#[cfg(test)]
mod architecture_tests;

mod graph_registration;
mod payload_registration;
mod registry;

pub use graph_registration::{GraphNodeRegistration, graph_node_registrations};
pub use payload_registration::{
    PayloadRegistration, PayloadRequestConfigurator, payload_registrations,
};
pub use registry::GraphRegistry;
