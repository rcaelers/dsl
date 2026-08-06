//! Generic typed-stream graph execution and scheduling.
//!
//! This owner maintains runtime graph wiring, channel lifecycle, node execution,
//! and pipeline supervision. Host work scheduling and worker-operation contracts
//! belong to `platform_runtime`; this crate consumes those capabilities but does
//! not own their adapters. It does not own capture storage, indexing, acquisition,
//! or presentation behavior.

#[cfg(test)]
mod architecture_tests;

mod app_manager;
mod cooperative_manager;
mod errors;
mod graph;
mod manager;
mod node;
mod payload_negotiation;
mod pipeline;
mod ports;
mod process_node_construction;
mod protocol;
mod receiver;
mod scheduler;
mod sender;
mod type_registry;
mod watchdog;

pub use app_manager::{
    AppManager, AppManagerBackend, AppManagerFactory, CooperativeAppManagerBackend,
    CooperativeAppManagerFactory, PipelineAppManagerFactory,
};
pub use cooperative_manager::CooperativeManager;
pub use errors::{ConnectionError, PortError, WorkError, WorkResult};
pub use graph::{Connection, GraphBuilder, NodeId};
pub use manager::{DisconnectEvent, InputSub, NodeFailure, NodeSpec, PipelineManager};
pub use node::{
    ConfigOutcome, ConfigValue, ConfigurationBoundary, ConfigurationScheduler,
    InputProtocolCandidate, InputScheduling, NodeCancellation, NodeConfig, ProcessNode,
    RuntimeExecutionMode, WorkOutcome,
};
pub use pipeline::Pipeline;
pub use ports::{
    InputPort, OutputPort, PortDirection, PortPayload, PortSchema, StreamSemantics, register_type,
};
pub use process_node_construction::ProcessNodeConstruction;
pub use protocol::{ProtocolCapability, ProtocolKind};
pub use receiver::{Receiver, ReceiverSelector};
pub use scheduler::{Scheduler, StopHandle};
pub use sender::{ChannelMessage, OverflowPolicy, Sender, SharedSenders};
pub use watchdog::{Watchdog, WatchdogHandle};
