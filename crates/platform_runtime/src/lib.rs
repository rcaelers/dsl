//! Platform-neutral work scheduling and worker-operation contracts.
//!
//! This crate owns host execution capabilities, serializable worker-operation
//! envelopes, portable fallback implementations, and the target-independent
//! queue used by native and browser worker adapters. It does not own stream
//! graphs, processing nodes, application policy, or target-specific transports.

mod work_executor;
mod worker_operation_queue;

pub use work_executor::{
    CompletedWorkTask, CooperativeWorkerOperationExecutor, InlineWorkExecutor, WorkExecutor,
    WorkExecutorTask, WorkTask, WorkerExecutionCapability, WorkerExecutionMode,
    WorkerKernelRegistry, WorkerMessage, WorkerMessageError, WorkerOperation,
    WorkerOperationExecutor, WorkerRequest,
};
pub use worker_operation_queue::{WorkerHostCommand, WorkerOperationQueue};
