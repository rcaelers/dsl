//! Generic typed-stream execution, scheduling, and work dispatch.
//!
//! This owner maintains runtime graph wiring, channel lifecycle, node execution,
//! and worker dispatch. It may consume generic signal contracts supplied by the
//! adjacent capture and derived-data owners, but it does not own their storage,
//! indexing, acquisition, or presentation behavior.

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
mod protocol;
mod receiver;
mod scheduler;
mod sender;
mod type_registry;
mod watchdog;
mod work_executor;
mod worker_operation_queue;

pub use app_manager::{
    AppManager, AppManagerBackend, AppManagerFactory, CooperativeAppManagerBackend,
    CooperativeAppManagerFactory,
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
pub use protocol::{ProtocolCapability, ProtocolKind};
pub use receiver::{Receiver, ReceiverSelector};
pub use scheduler::{Scheduler, StopHandle};
pub use sender::{ChannelMessage, OverflowPolicy, Sender, SharedSenders};
pub use watchdog::{Watchdog, WatchdogHandle};
pub use work_executor::{
    CompletedWorkTask, CooperativeWorkerOperationExecutor, InlineWorkExecutor, WorkExecutor,
    WorkExecutorTask, WorkTask, WorkerExecutionCapability, WorkerExecutionMode,
    WorkerKernelRegistry, WorkerMessage, WorkerMessageError, WorkerOperation,
    WorkerOperationExecutor, WorkerRequest,
};
pub use worker_operation_queue::{WorkerHostCommand, WorkerOperationQueue};
