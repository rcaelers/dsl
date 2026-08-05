//! Portable dynamic node-catalog service owned by application composition.

mod contract;

#[cfg(test)]
mod architecture_tests;

pub use contract::{NodeCatalogService, NodeCatalogSnapshot};
