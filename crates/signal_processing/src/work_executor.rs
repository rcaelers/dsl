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
    use super::{WorkerMessage, WorkerOperation, WorkerRequest};

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
}
