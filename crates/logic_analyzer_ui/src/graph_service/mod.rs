//! UI-owned graph discovery, execution, and source-preparation service.

#[cfg(test)]
mod architecture_tests;
mod contract;
mod graph_compiler;
#[cfg(test)]
mod graph_service_tests;

pub(crate) use contract::{GraphRun, GraphService};
pub(crate) use graph_compiler::{
    graph_service_with_execution, graph_service_with_execution_and_builder_overrides,
    standard_graph_service,
};
