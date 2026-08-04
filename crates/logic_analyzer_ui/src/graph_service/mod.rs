//! UI-owned graph discovery, execution, and source-preparation service.

#[cfg(test)]
mod architecture_tests;
mod composition;
mod contract;
#[cfg(test)]
mod graph_service_tests;

pub(crate) use composition::{
    graph_service_with_execution, graph_service_with_execution_and_builder_overrides,
    standard_graph_service,
};
pub(crate) use contract::{GraphRun, GraphService};
