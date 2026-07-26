//! UI-owned graph discovery, execution, and source-preparation service.

#[cfg(test)]
mod architecture_tests;
mod contract;
mod graph_compiler;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod graph_service_tests;
#[cfg(not(target_arch = "wasm32"))]
#[path = "platform_contract_native.rs"]
mod platform_contract;
#[cfg(target_arch = "wasm32")]
#[path = "platform_contract_wasm.rs"]
mod platform_contract;
#[cfg(not(target_arch = "wasm32"))]
#[path = "platform_graph_compiler_native.rs"]
mod platform_graph_compiler;
#[cfg(target_arch = "wasm32")]
#[path = "platform_graph_compiler_wasm.rs"]
mod platform_graph_compiler;

pub(crate) use contract::{GraphRun, GraphService};
pub(crate) use graph_compiler::standard_graph_service;
