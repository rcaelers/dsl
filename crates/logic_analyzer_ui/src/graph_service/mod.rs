//! UI-owned graph discovery, execution, and source-preparation service.

mod contract;
mod graph_compiler;
#[cfg(test)]
mod graph_service_tests;
mod revision_preparation;

pub(crate) use contract::{GraphRun, GraphRunFailure};
pub(crate) use graph_compiler::{
    UiGraphService, graph_service_with_execution,
    graph_service_with_execution_and_capability_overrides, standard_graph_service,
};
pub(crate) use revision_preparation::{GraphRevisionPreparationTask, PreparedGraphRevision};
