use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::worker_operation_queue::WorkerQueueError;

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
    ///
    /// # Parameters
    /// - `value`: Non-empty stable operation identifier owned by a kernel.
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
    Cancel {
        /// Caller-assigned sequence of the operation to cancel.
        sequence: u64,
    },
    /// Reports monotonic operation progress when its total is known.
    Progress {
        /// Caller-assigned sequence of the active operation.
        sequence: u64,
        /// Completed work units reported by the operation.
        completed: u64,
        /// Total work units, when the operation can determine one.
        total: Option<u64>,
    },
    /// Returns the owned result payload for one completed operation.
    Complete {
        /// Caller-assigned sequence of the completed operation.
        sequence: u64,
        /// Owned result bytes interpreted by the operation owner.
        payload: Vec<u8>,
    },
    /// Reports an operation failure without leaking host-specific error types.
    Failed {
        /// Caller-assigned sequence of the failed operation.
        sequence: u64,
        /// Classified operation, host, or protocol failure.
        error: WorkerFailure,
    },
}

/// Validation error for the portable worker-message envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkerMessageError {
    #[error("worker operation identifiers cannot be empty")]
    InvalidOperation,
    #[error("worker operation is already registered")]
    DuplicateOperation,
}

/// Failure returned by a registered finite worker kernel.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkerKernelError {
    /// The operation owner rejected the payload or could not produce its result.
    #[error("{message}")]
    Execution {
        /// Owner-provided operation diagnostic.
        message: String,
    },
}

impl From<String> for WorkerKernelError {
    fn from(message: String) -> Self {
        Self::Execution { message }
    }
}

impl From<&str> for WorkerKernelError {
    fn from(message: &str) -> Self {
        Self::Execution {
            message: message.to_owned(),
        }
    }
}

/// Terminal failure transported in a [`WorkerMessage`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum WorkerFailure {
    /// The request names no kernel in the selected registry.
    #[error("worker operation '{operation}' is not registered")]
    OperationNotRegistered {
        /// Stable operation identifier requested by the caller.
        operation: String,
    },
    /// The registered operation rejected its payload or failed while executing.
    #[error("{message}")]
    Kernel {
        /// Operation-owner diagnostic.
        message: String,
    },
    /// The selected host mechanism failed.
    #[error("{message}")]
    Host {
        /// Host-adapter diagnostic with no host-specific error type.
        message: String,
    },
    /// A worker violated the portable request/response protocol.
    #[error("{message}")]
    Protocol {
        /// Protocol diagnostic.
        message: String,
    },
    /// The operation was cancelled before its result was released.
    #[error("worker operation was cancelled")]
    Cancelled,
    /// No worker remains available to execute accepted work.
    #[error("all workers are unavailable")]
    Unavailable,
}

impl From<WorkerKernelError> for WorkerFailure {
    fn from(error: WorkerKernelError) -> Self {
        match error {
            WorkerKernelError::Execution { message } => Self::Kernel { message },
        }
    }
}

type WorkerKernel = dyn Fn(Vec<u8>) -> Result<Vec<u8>, WorkerKernelError> + Send + Sync + 'static;

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
    ///
    /// # Parameters
    /// - `parallelism`: Maximum independently scheduled operations.
    /// - `operations`: Stable operation identifiers supported by the host.
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
    fn submit(&self, request: WorkerRequest) -> Result<(), WorkerQueueError>;

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
    ///
    /// # Parameters
    /// - `kernels`: Portable operation catalog executed by the fallback.
    /// - `unavailable_reason`: Reason a parallel host was not selected.
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

    fn submit(&self, request: WorkerRequest) -> Result<(), WorkerQueueError> {
        let mut last_sequence = self.last_sequence.borrow_mut();
        if last_sequence.is_some_and(|previous| request.sequence <= previous) {
            return Err(WorkerQueueError::NonMonotonicSequence {
                sequence: request.sequence,
                previous: last_sequence.unwrap_or_default(),
            });
        }
        if !self.capability.supports(&request.operation) {
            return Err(WorkerQueueError::OperationNotRegistered {
                operation: request.operation.as_str().to_owned(),
            });
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
        F: Fn(Vec<u8>) -> Result<Vec<u8>, WorkerKernelError> + Send + Sync + 'static,
    {
        let operation = WorkerOperation::new(operation)?;
        if self.kernels.contains_key(&operation) {
            return Err(WorkerMessageError::DuplicateOperation);
        }
        self.kernels.insert(operation, Arc::new(kernel));
        Ok(())
    }

    /// Reports whether the catalog can execute `operation`.
    ///
    /// # Parameters
    /// - `operation`: Stable operation identifier to test.
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
                error: WorkerFailure::OperationNotRegistered {
                    operation: request.operation.as_str().to_owned(),
                },
            };
        };
        match kernel(request.payload) {
            Ok(payload) => WorkerMessage::Complete { sequence, payload },
            Err(error) => WorkerMessage::Failed {
                sequence,
                error: error.into(),
            },
        }
    }
}

/// Failure to submit work through a [`WorkExecutor`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkExecutorError {
    /// The bounded finite-work queue has reached capacity.
    #[error("processing work executor queue is full")]
    QueueFull,
    /// The host work executor is no longer running.
    #[error("processing work executor stopped")]
    Stopped,
    /// The host could not start an independently scheduled task.
    #[error("{message}")]
    TaskStart {
        /// Host-neutral task-start diagnostic.
        message: String,
    },
    /// An executor implementation rejected the task for another stated reason.
    #[error("{message}")]
    Rejected {
        /// Executor-owned rejection diagnostic.
        message: String,
    },
}

impl From<String> for WorkExecutorError {
    fn from(message: String) -> Self {
        Self::Rejected { message }
    }
}

impl From<&str> for WorkExecutorError {
    fn from(message: &str) -> Self {
        Self::Rejected {
            message: message.to_owned(),
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

    /// Whether the host can schedule a blocking or long-lived task without
    /// blocking the cooperative caller.
    fn supports_long_running_tasks(&self) -> bool {
        false
    }

    /// Gives the host an opportunity to back off a quiet long-running task.
    /// Cooperative hosts return immediately; threaded hosts may sleep.
    fn idle(&self, _duration: Duration) {}

    /// Enqueues finite work without exposing worker implementation details.
    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, WorkExecutorError>;

    /// Enqueues finite work with an owner-defined diagnostic label.
    ///
    /// Hosts must not select execution behavior from this label. Profilers may
    /// use it to attribute otherwise opaque work while ordinary executors
    /// preserve the same behavior as [`Self::submit`].
    fn submit_labeled(
        &self,
        _label: &'static str,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
        self.submit(task)
    }

    /// Starts long-lived work that may block on runtime stream endpoints.
    ///
    /// Hosts with a bounded finite-work queue override this to keep a slow
    /// reader, processing node, or watchdog from starving indexing work.
    fn submit_long_running(
        &self,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
        self.submit(task)
    }

    /// Starts long-lived work with an owner-defined diagnostic label.
    ///
    /// As with [`Self::submit_labeled`], hosts must not select behavior from
    /// the label.
    fn submit_long_running_labeled(
        &self,
        _label: &'static str,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
        self.submit_long_running(task)
    }
}

/// Portable executor that performs work on the calling cooperative pump.
pub struct InlineWorkExecutor;

impl WorkExecutor for InlineWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
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
        CooperativeWorkerOperationExecutor, WorkExecutorError, WorkerExecutionMode, WorkerFailure,
        WorkerKernelError, WorkerKernelRegistry, WorkerMessage, WorkerMessageError,
        WorkerOperation, WorkerOperationExecutor, WorkerRequest,
    };

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn worker_messages_round_trip_as_owned_data() {
        let above_wasm32 = u64::from(u32::MAX) + 53;
        let messages = [
            WorkerMessage::Run(WorkerRequest {
                sequence: u64::MAX,
                operation: WorkerOperation::new("org.example.encode-block/v1").unwrap(),
                payload: vec![0, 1, 2, 255],
            }),
            WorkerMessage::Cancel { sequence: 2 },
            WorkerMessage::Progress {
                sequence: above_wasm32,
                completed: above_wasm32 + 1,
                total: Some(above_wasm32 + 2),
            },
            WorkerMessage::Progress {
                sequence: 4,
                completed: 13,
                total: None,
            },
            WorkerMessage::Complete {
                sequence: 5,
                payload: vec![9, 8, 7],
            },
            WorkerMessage::Failed {
                sequence: 6,
                error: WorkerFailure::Kernel {
                    message: "expected failure".to_string(),
                },
            },
        ];

        for message in messages {
            let encoded = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                serde_json::from_slice::<WorkerMessage>(&encoded).unwrap(),
                message
            );
        }
    }

    #[test]
    fn worker_operations_require_a_stable_nonempty_identifier() {
        assert_eq!(
            WorkerOperation::new("  ").unwrap_err(),
            WorkerMessageError::InvalidOperation
        );
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
        assert_eq!(
            registry.register("org.example.reverse/v1", Ok).unwrap_err(),
            WorkerMessageError::DuplicateOperation
        );

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

        registry
            .register("org.example.fail/v1", |_| {
                Err(WorkerKernelError::Execution {
                    message: "expected kernel failure".to_string(),
                })
            })
            .unwrap();
        assert_eq!(
            registry.execute(WorkerRequest {
                sequence: 43,
                operation: WorkerOperation::new("org.example.fail/v1").unwrap(),
                payload: Vec::new(),
            }),
            WorkerMessage::Failed {
                sequence: 43,
                error: WorkerFailure::Kernel {
                    message: "expected kernel failure".to_string(),
                },
            }
        );
    }

    #[test]
    fn work_executor_errors_preserve_classification_and_diagnostics() {
        assert_eq!(
            WorkExecutorError::QueueFull.to_string(),
            "processing work executor queue is full"
        );
        assert_eq!(
            WorkExecutorError::TaskStart {
                message: "host refused task".to_string(),
            }
            .to_string(),
            "host refused task"
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
