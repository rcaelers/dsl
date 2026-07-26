//! Concrete, UI-independent logic-analyzer processing nodes.

#[cfg(test)]
mod architecture_tests;

pub mod nodes;
#[cfg(not(target_arch = "wasm32"))]
mod support;
pub mod types;
