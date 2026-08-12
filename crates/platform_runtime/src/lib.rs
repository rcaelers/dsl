//! Platform-neutral host-runtime capability contracts.
//!
//! This crate owns scoped system-activity leases, host execution capabilities,
//! serializable worker-operation envelopes, portable fallback implementations,
//! and the target-independent queue used by native and browser worker adapters.
//! It does not own stream graphs, processing nodes, application policy, or
//! target-specific transports.

mod system_activity;
mod work_executor;
mod worker_operation_queue;

pub use system_activity::{
    ObservedSystemActivityLease, ObservedSystemActivityManager, SystemActivityInterruption,
    SystemActivityLease, SystemActivityManager,
};
pub use work_executor::{
    CompletedWorkTask, CooperativeWorkerOperationExecutor, InlineWorkExecutor, WorkExecutor,
    WorkExecutorError, WorkExecutorTask, WorkTask, WorkerExecutionCapability, WorkerExecutionMode,
    WorkerFailure, WorkerKernelError, WorkerKernelRegistry, WorkerMessage, WorkerMessageError,
    WorkerOperation, WorkerOperationExecutor, WorkerRequest,
};
pub use worker_operation_queue::{WorkerHostCommand, WorkerOperationQueue, WorkerQueueError};
