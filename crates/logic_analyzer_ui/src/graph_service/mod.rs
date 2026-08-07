//! UI-owned graph discovery, execution, and source-preparation service.

mod contract;
mod graph_compiler;
#[cfg(test)]
mod graph_service_tests;

pub(crate) use contract::GraphRun;
pub(crate) use graph_compiler::{
    UiGraphService, graph_service_with_execution,
    graph_service_with_execution_and_capability_overrides, standard_graph_service,
};
