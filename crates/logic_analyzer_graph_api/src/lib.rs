//! Compile-time extension contracts for graph nodes and payload plugins.
//!
//! Plugins implement or submit contracts from [`node`], and use [`node_support`]
//! for the associated values and restricted build context. This crate owns neither
//! built-in nodes, graph lowering or execution, UI state, viewer widgets, capture
//! export, nor target selection. Concrete node migration remains with the node
//! feature that owns the serialized state.

#[cfg(test)]
mod architecture_tests;
pub mod node;
pub mod node_support;
