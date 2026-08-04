//! Application-neutral orchestration that composes graph compilation and runtime execution.

mod worker_client;
mod worker_execution;
mod worker_execution_codec;

#[cfg(test)]
mod architecture_tests;

pub use worker_client::GraphWorkerClient;
pub use worker_execution::{GraphWorkerMessage, GraphWorkerRequest, GraphWorkerRuntime};
pub use worker_execution_codec::{
    decode_graph_worker_messages, decode_graph_worker_request, encode_graph_worker_messages,
    encode_graph_worker_request,
};
