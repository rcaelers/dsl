use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// One bounded unit of host-scheduled work.
pub type WorkExecutorTask = Box<dyn FnOnce() + Send + 'static>;

/// Stable identifier for a worker operation that can run outside the current
/// Rust instance.
///
/// The identifier belongs to the operation owner, not to a host, thread, or
/// browser implementation. Its payload is interpreted only by a worker that
/// explicitly registers the same operation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkerOperation(String);

impl WorkerOperation {
    /// Creates an operation identifier suitable for a serialized worker
    /// request.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkerMessageError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WorkerMessageError::InvalidOperation);
        }
        Ok(Self(value))
    }

    /// Returns the operation's stable identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A serializable request for one independently executable worker operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRequest {
    /// Monotonic caller-assigned sequence used to merge completions in submit
    /// order regardless of worker completion order.
    pub sequence: u64,
    /// The registered operation that interprets `payload`.
    pub operation: WorkerOperation,
    /// Owned operation input. Hosts may transfer its backing without exposing
    /// a browser or native buffer type to the operation contract.
    pub payload: Vec<u8>,
}

/// A serializable worker-to-host message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// Requests execution of one finite operation.
    Run(WorkerRequest),
    /// Requests cancellation of an outstanding operation.
    Cancel { sequence: u64 },
    /// Reports monotonic operation progress when its total is known.
    Progress {
        sequence: u64,
        completed: u64,
        total: Option<u64>,
    },
    /// Returns the owned result payload for one completed operation.
    Complete { sequence: u64, payload: Vec<u8> },
    /// Reports an operation failure without leaking host-specific error types.
    Failed { sequence: u64, message: String },
}

/// Validation error for the portable worker-message envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkerMessageError {
    #[error("worker operation identifiers cannot be empty")]
    InvalidOperation,
    #[error("worker operation is already registered")]
    DuplicateOperation,
}

type WorkerKernel = dyn Fn(Vec<u8>) -> Result<Vec<u8>, String> + Send + Sync + 'static;

/// Portable catalog of finite operations available to a host worker.
///
/// The platform adapter only transports [`WorkerMessage`] values and invokes
/// this registry. Operation owners retain the binary payload codecs and
/// algorithms behind their stable identifiers.
#[derive(Clone, Default)]
pub struct WorkerKernelRegistry {
    kernels: BTreeMap<WorkerOperation, Arc<WorkerKernel>>,
}

/// Execution mode selected for serializable finite operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerExecutionMode {
    /// Operations run on the cooperative caller.
    Cooperative,
    /// Operations run in an independently scheduled worker pool.
    Parallel,
}

/// Immutable description of the selected finite-operation host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerExecutionCapability {
    mode: WorkerExecutionMode,
    parallelism: usize,
    operations: Vec<WorkerOperation>,
    unavailable_reason: Option<String>,
}

impl WorkerExecutionCapability {
    /// Creates a capability for an active parallel worker host.
    pub fn parallel(parallelism: usize, mut operations: Vec<WorkerOperation>) -> Self {
        operations.sort();
        operations.dedup();
        Self {
            mode: WorkerExecutionMode::Parallel,
            parallelism: parallelism.max(1),
            operations,
            unavailable_reason: None,
        }
    }

    /// Creates a capability for the portable cooperative fallback.
    pub fn cooperative(
        mut operations: Vec<WorkerOperation>,
        unavailable_reason: impl Into<String>,
    ) -> Self {
        operations.sort();
        operations.dedup();
        Self {
            mode: WorkerExecutionMode::Cooperative,
            parallelism: 1,
            operations,
            unavailable_reason: Some(unavailable_reason.into()),
        }
    }

    /// Selected execution mode.
    pub fn mode(&self) -> WorkerExecutionMode {
        self.mode
    }

    /// Advertised independent operation capacity.
    pub fn parallelism(&self) -> usize {
        self.parallelism
    }

    /// Stable operation identifiers supported by this host.
    pub fn operations(&self) -> &[WorkerOperation] {
        &self.operations
    }

    /// Why parallel worker execution is unavailable, if cooperative fallback
    /// was selected.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    /// Whether `operation` is supported by the selected host.
    pub fn supports(&self, operation: &WorkerOperation) -> bool {
        self.operations.binary_search(operation).is_ok()
    }
}

/// Host for finite serializable operations.
///
/// This is distinct from [`WorkExecutor`]: closures and long-running runtime
/// tasks remain cooperative on a single-threaded browser, while registered
/// owned operations can cross a worker boundary.
pub trait WorkerOperationExecutor {
    /// Returns the immutable capability selected at composition time.
    fn capability(&self) -> WorkerExecutionCapability;

    /// Submits one monotonically sequenced finite operation.
    fn submit(&self, request: WorkerRequest) -> Result<(), String>;

    /// Cancels queued or running work when possible.
    fn cancel(&self, sequence: u64) -> bool;

    /// Drains progress and terminal messages.
    fn drain_messages(&self) -> Vec<WorkerMessage>;

    /// Number of accepted requests without a released terminal result.
    fn outstanding(&self) -> usize;
}

/// Portable fallback that executes registered operations on submission.
pub struct CooperativeWorkerOperationExecutor {
    kernels: WorkerKernelRegistry,
    capability: WorkerExecutionCapability,
    messages: RefCell<VecDeque<WorkerMessage>>,
    last_sequence: RefCell<Option<u64>>,
}

impl CooperativeWorkerOperationExecutor {
    /// Creates a fallback for a host that cannot provide parallel workers.
    pub fn new(kernels: WorkerKernelRegistry, unavailable_reason: impl Into<String>) -> Self {
        let operations = kernels.operations().cloned().collect::<Vec<_>>();
        Self {
            kernels,
            capability: WorkerExecutionCapability::cooperative(operations, unavailable_reason),
            messages: RefCell::new(VecDeque::new()),
            last_sequence: RefCell::new(None),
        }
    }
}

impl WorkerOperationExecutor for CooperativeWorkerOperationExecutor {
    fn capability(&self) -> WorkerExecutionCapability {
        self.capability.clone()
    }

    fn submit(&self, request: WorkerRequest) -> Result<(), String> {
        let mut last_sequence = self.last_sequence.borrow_mut();
        if last_sequence.is_some_and(|previous| request.sequence <= previous) {
            return Err(format!(
                "worker request sequence {} is not greater than the previous sequence",
                request.sequence
            ));
        }
        if !self.capability.supports(&request.operation) {
            return Err(format!(
                "worker operation '{}' is not registered",
                request.operation.as_str()
            ));
        }
        *last_sequence = Some(request.sequence);
        let sequence = request.sequence;
        let mut messages = self.messages.borrow_mut();
        messages.push_back(WorkerMessage::Progress {
            sequence,
            completed: 0,
            total: Some(1),
        });
        let terminal = self.kernels.execute(request);
        messages.push_back(WorkerMessage::Progress {
            sequence,
            completed: 1,
            total: Some(1),
        });
        messages.push_back(terminal);
        Ok(())
    }

    fn cancel(&self, _sequence: u64) -> bool {
        false
    }

    fn drain_messages(&self) -> Vec<WorkerMessage> {
        self.messages.borrow_mut().drain(..).collect()
    }

    fn outstanding(&self) -> usize {
        0
    }
}

impl WorkerKernelRegistry {
    /// Creates an empty operation catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one finite operation.
    pub fn register<F>(
        &mut self,
        operation: impl Into<String>,
        kernel: F,
    ) -> Result<(), WorkerMessageError>
    where
        F: Fn(Vec<u8>) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    {
        let operation = WorkerOperation::new(operation)?;
        if self.kernels.contains_key(&operation) {
            return Err(WorkerMessageError::DuplicateOperation);
        }
        self.kernels.insert(operation, Arc::new(kernel));
        Ok(())
    }

    /// Reports whether the catalog can execute `operation`.
    pub fn supports(&self, operation: &WorkerOperation) -> bool {
        self.kernels.contains_key(operation)
    }

    /// Lists registered stable operation identifiers.
    pub fn operations(&self) -> impl Iterator<Item = &WorkerOperation> {
        self.kernels.keys()
    }

    /// Executes one request and preserves its sequence in the completion.
    pub fn execute(&self, request: WorkerRequest) -> WorkerMessage {
        let sequence = request.sequence;
        let Some(kernel) = self.kernels.get(&request.operation) else {
            return WorkerMessage::Failed {
                sequence,
                message: format!(
                    "worker operation '{}' is not registered",
                    request.operation.as_str()
                ),
            };
        };
        match kernel(request.payload) {
            Ok(payload) => WorkerMessage::Complete { sequence, payload },
            Err(message) => WorkerMessage::Failed { sequence, message },
        }
    }
}

/// Completion handle for one host-scheduled work unit.
pub trait WorkTask: Send {
    /// Whether the submitted work has completed.
    fn is_finished(&self) -> bool;

    /// Waits until the submitted work completes.
    fn wait(self: Box<Self>);
}

/// Host execution capability used by portable processing nodes.
pub trait WorkExecutor: Send + Sync {
    /// Number of independent tasks the host can run concurrently.
    fn available_parallelism(&self) -> usize;

    /// Enqueues finite work without exposing worker implementation details.
    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String>;

    /// Starts long-lived work that may block on runtime stream endpoints.
    ///
    /// Hosts with a bounded finite-work queue override this to keep a slow
    /// reader, processing node, or watchdog from starving indexing work.
    fn submit_long_running(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        self.submit(task)
    }
}

/// Portable executor that performs work on the calling cooperative pump.
pub struct InlineWorkExecutor;

impl WorkExecutor for InlineWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        task();
        Ok(Box::new(CompletedWorkTask))
    }
}

/// Completion handle for work that already ran synchronously.
pub struct CompletedWorkTask;

impl WorkTask for CompletedWorkTask {
    fn is_finished(&self) -> bool {
        true
    }

    fn wait(self: Box<Self>) {}
}

#[cfg(test)]
mod work_executor_tests {
    use super::{
        CooperativeWorkerOperationExecutor, WorkerExecutionMode, WorkerKernelRegistry,
        WorkerMessage, WorkerOperation, WorkerOperationExecutor, WorkerRequest,
    };

    #[test]
    fn worker_messages_round_trip_as_owned_data() {
        let message = WorkerMessage::Run(WorkerRequest {
            sequence: u64::MAX,
            operation: WorkerOperation::new("signal-processing.encode-word-block/v1").unwrap(),
            payload: vec![1, 2, 3],
        });

        let encoded = serde_json::to_vec(&message).unwrap();
        assert_eq!(
            serde_json::from_slice::<WorkerMessage>(&encoded).unwrap(),
            message
        );
    }

    #[test]
    fn worker_operations_require_a_stable_nonempty_identifier() {
        assert!(WorkerOperation::new("  ").is_err());
    }

    #[test]
    fn kernel_registry_preserves_sequence_and_rejects_duplicates() {
        let mut registry = WorkerKernelRegistry::new();
        registry
            .register("org.example.reverse/v1", |mut payload| {
                payload.reverse();
                Ok(payload)
            })
            .unwrap();
        assert!(registry.register("org.example.reverse/v1", Ok).is_err());

        let operation = WorkerOperation::new("org.example.reverse/v1").unwrap();
        assert!(registry.supports(&operation));
        assert_eq!(
            registry.execute(WorkerRequest {
                sequence: 42,
                operation,
                payload: vec![1, 2, 3],
            }),
            WorkerMessage::Complete {
                sequence: 42,
                payload: vec![3, 2, 1],
            }
        );
    }

    #[test]
    fn cooperative_operation_host_preserves_behavior_when_parallelism_is_unavailable() {
        let mut kernels = WorkerKernelRegistry::new();
        kernels
            .register("org.example.reverse/v1", |mut payload| {
                payload.reverse();
                Ok(payload)
            })
            .unwrap();
        let executor = CooperativeWorkerOperationExecutor::new(kernels, "workers unavailable");
        let operation = WorkerOperation::new("org.example.reverse/v1").unwrap();

        assert_eq!(
            executor.capability().mode(),
            WorkerExecutionMode::Cooperative
        );
        assert_eq!(
            executor.capability().unavailable_reason(),
            Some("workers unavailable")
        );
        executor
            .submit(WorkerRequest {
                sequence: 10,
                operation,
                payload: vec![1, 2, 3],
            })
            .unwrap();

        assert_eq!(
            executor.drain_messages(),
            vec![
                WorkerMessage::Progress {
                    sequence: 10,
                    completed: 0,
                    total: Some(1),
                },
                WorkerMessage::Progress {
                    sequence: 10,
                    completed: 1,
                    total: Some(1),
                },
                WorkerMessage::Complete {
                    sequence: 10,
                    payload: vec![3, 2, 1],
                },
            ]
        );
    }

    #[test]
    fn worker_capabilities_normalize_registered_operations() {
        let alpha = WorkerOperation::new("org.example.alpha/v1").unwrap();
        let beta = WorkerOperation::new("org.example.beta/v1").unwrap();
        let capability = super::WorkerExecutionCapability::parallel(
            2,
            vec![beta.clone(), alpha.clone(), beta.clone()],
        );

        assert_eq!(capability.operations(), &[alpha.clone(), beta]);
        assert!(capability.supports(&alpha));
    }
}
