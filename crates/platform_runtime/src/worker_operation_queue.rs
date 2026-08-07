use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::work_executor::{
    WorkerExecutionCapability, WorkerFailure, WorkerMessage, WorkerOperation, WorkerRequest,
};

/// Admission or configuration failure for a bounded worker-operation queue.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkerQueueError {
    /// A worker pool cannot execute requests without at least one host slot.
    #[error("the worker pool must contain at least one worker")]
    EmptyPool,
    /// The outstanding-request bound cannot keep every worker occupied.
    #[error("the worker queue must hold at least one request per worker")]
    InsufficientCapacity {
        /// Configured number of worker slots.
        worker_count: usize,
        /// Configured maximum number of outstanding requests.
        max_outstanding: usize,
    },
    /// The selected worker host does not advertise the requested operation.
    #[error("worker operation '{operation}' is not registered")]
    OperationNotRegistered {
        /// Stable operation identifier requested by the caller.
        operation: String,
    },
    /// A caller reused or moved backwards in its monotonically increasing sequence.
    #[error("worker request sequence {sequence} is not greater than the previous sequence")]
    NonMonotonicSequence {
        /// Rejected caller sequence.
        sequence: u64,
        /// Most recently accepted sequence.
        previous: u64,
    },
    /// The bounded queue already contains its maximum accepted work.
    #[error("worker request queue is full")]
    Full,
}

#[derive(Clone, Debug, Default)]
struct WorkerState {
    ready: bool,
    failed: bool,
    running: Option<u64>,
}

/// Command produced by the portable finite-operation queue for a host adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerHostCommand {
    /// Submit an owned request to one host worker.
    Run {
        /// Stable zero-based worker slot.
        worker_index: usize,
        /// Request transferred to the worker.
        request: WorkerRequest,
    },
    /// Ask one host worker to cancel its active request.
    Cancel {
        /// Stable zero-based worker slot.
        worker_index: usize,
        /// Sequence assigned to the active request.
        sequence: u64,
    },
}

/// Target-independent scheduler for a bounded finite-operation worker pool.
///
/// Host adapters translate [`WorkerHostCommand`] values into their native
/// transport and report readiness, progress, completion, and failure back to
/// this queue. Completion messages are released in submission order even when
/// workers finish out of order.
pub struct WorkerOperationQueue {
    workers: Vec<WorkerState>,
    pending: VecDeque<WorkerRequest>,
    submission_order: VecDeque<u64>,
    terminal: BTreeMap<u64, WorkerMessage>,
    delivered: VecDeque<WorkerMessage>,
    cancelled: BTreeSet<u64>,
    max_outstanding: usize,
    last_submitted_sequence: Option<u64>,
    operations: BTreeSet<WorkerOperation>,
}

impl WorkerOperationQueue {
    /// Creates a bounded scheduler for `worker_count` equivalent host slots.
    ///
    /// # Parameters
    /// - `worker_count`: Input consumed by this operation.
    /// - `max_outstanding`: Input consumed by this operation.
    /// - `operations`: Input consumed by this operation.
    pub fn new(
        worker_count: usize,
        max_outstanding: usize,
        operations: impl IntoIterator<Item = WorkerOperation>,
    ) -> Result<Self, WorkerQueueError> {
        if worker_count == 0 {
            return Err(WorkerQueueError::EmptyPool);
        }
        if max_outstanding < worker_count {
            return Err(WorkerQueueError::InsufficientCapacity {
                worker_count,
                max_outstanding,
            });
        }
        Ok(Self {
            workers: vec![WorkerState::default(); worker_count],
            pending: VecDeque::new(),
            submission_order: VecDeque::new(),
            terminal: BTreeMap::new(),
            delivered: VecDeque::new(),
            cancelled: BTreeSet::new(),
            max_outstanding,
            last_submitted_sequence: None,
            operations: operations.into_iter().collect(),
        })
    }

    /// Immutable description of the available parallel worker host.
    pub fn capability(&self) -> WorkerExecutionCapability {
        WorkerExecutionCapability::parallel(
            self.workers.len(),
            self.operations.iter().cloned().collect(),
        )
    }

    /// Number of host worker slots.
    pub fn available_parallelism(&self) -> usize {
        self.workers.len()
    }

    /// Number of accepted requests whose ordered terminal result is pending.
    pub fn outstanding(&self) -> usize {
        self.submission_order.len()
    }

    /// Accepts a monotonically sequenced request and returns runnable work.
    pub fn submit(
        &mut self,
        request: WorkerRequest,
    ) -> Result<Vec<WorkerHostCommand>, WorkerQueueError> {
        if !self.operations.contains(&request.operation) {
            return Err(WorkerQueueError::OperationNotRegistered {
                operation: request.operation.as_str().to_owned(),
            });
        }
        if self
            .last_submitted_sequence
            .is_some_and(|previous| request.sequence <= previous)
        {
            return Err(WorkerQueueError::NonMonotonicSequence {
                sequence: request.sequence,
                previous: self.last_submitted_sequence.unwrap_or_default(),
            });
        }
        if self.outstanding() >= self.max_outstanding {
            return Err(WorkerQueueError::Full);
        }
        self.last_submitted_sequence = Some(request.sequence);
        self.submission_order.push_back(request.sequence);
        self.pending.push_back(request);
        Ok(self.dispatch_ready_workers())
    }

    /// Marks one initialized host worker ready and returns runnable work.
    pub fn worker_ready(&mut self, worker_index: usize) -> Vec<WorkerHostCommand> {
        if let Some(worker) = self.workers.get_mut(worker_index)
            && !worker.failed
        {
            worker.ready = true;
        }
        self.dispatch_ready_workers()
    }

    /// Records progress for an outstanding, non-cancelled request.
    ///
    /// # Parameters
    /// - `sequence`: Input consumed by this operation.
    /// - `completed`: Input consumed by this operation.
    /// - `total`: Input consumed by this operation.
    pub fn worker_progress(&mut self, sequence: u64, completed: u64, total: Option<u64>) {
        if self.contains_sequence(sequence) && !self.cancelled.contains(&sequence) {
            self.delivered.push_back(WorkerMessage::Progress {
                sequence,
                completed,
                total,
            });
        }
    }

    /// Records one worker's terminal result and returns newly runnable work.
    pub fn worker_completed(
        &mut self,
        worker_index: usize,
        reported_sequence: Option<u64>,
        result: Result<Vec<u8>, WorkerFailure>,
    ) -> Vec<WorkerHostCommand> {
        let Some(running) = self
            .workers
            .get(worker_index)
            .and_then(|worker| worker.running)
        else {
            return self.worker_failed(
                worker_index,
                WorkerFailure::Protocol {
                    message: "worker returned a terminal message without an active request"
                        .to_string(),
                },
            );
        };
        if reported_sequence.is_some_and(|sequence| sequence != running) {
            return self.worker_failed(
                worker_index,
                WorkerFailure::Protocol {
                    message: format!(
                        "worker returned sequence {} while running sequence {running}",
                        reported_sequence.unwrap_or_default()
                    ),
                },
            );
        }
        self.workers[worker_index].running = None;
        if self.contains_sequence(running) && !self.cancelled.contains(&running) {
            let message = match result {
                Ok(payload) => WorkerMessage::Complete {
                    sequence: running,
                    payload,
                },
                Err(error) => WorkerMessage::Failed {
                    sequence: running,
                    error,
                },
            };
            self.record_terminal(message);
        }
        self.dispatch_ready_workers()
    }

    /// Marks one host worker unavailable and returns work reassigned elsewhere.
    pub fn worker_failed(
        &mut self,
        worker_index: usize,
        error: WorkerFailure,
    ) -> Vec<WorkerHostCommand> {
        let Some(worker) = self.workers.get_mut(worker_index) else {
            return Vec::new();
        };
        worker.ready = false;
        worker.failed = true;
        if let Some(sequence) = worker.running.take()
            && !self.cancelled.contains(&sequence)
        {
            self.record_terminal(WorkerMessage::Failed { sequence, error });
        }
        self.fail_pending_if_unavailable();
        self.dispatch_ready_workers()
    }

    /// Cancels queued or active work and returns required host commands.
    pub fn cancel(&mut self, sequence: u64) -> (bool, Vec<WorkerHostCommand>) {
        if !self.contains_sequence(sequence) {
            return (false, Vec::new());
        }
        self.cancelled.insert(sequence);
        if let Some(index) = self
            .pending
            .iter()
            .position(|request| request.sequence == sequence)
        {
            self.pending.remove(index);
        }
        let mut commands = self
            .workers
            .iter()
            .enumerate()
            .filter(|(_, worker)| worker.running == Some(sequence))
            .map(|(worker_index, _)| WorkerHostCommand::Cancel {
                worker_index,
                sequence,
            })
            .collect::<Vec<_>>();
        self.record_terminal(WorkerMessage::Failed {
            sequence,
            error: WorkerFailure::Cancelled,
        });
        commands.extend(self.dispatch_ready_workers());
        (true, commands)
    }

    /// Drains progress and ordered terminal messages.
    pub fn drain_messages(&mut self) -> Vec<WorkerMessage> {
        self.delivered.drain(..).collect()
    }

    fn contains_sequence(&self, sequence: u64) -> bool {
        self.submission_order.contains(&sequence)
    }

    fn record_terminal(&mut self, message: WorkerMessage) {
        let sequence = terminal_sequence(&message);
        if self.contains_sequence(sequence) && !self.terminal.contains_key(&sequence) {
            self.terminal.insert(sequence, message);
        }
        self.release_ordered();
    }

    fn release_ordered(&mut self) {
        while let Some(sequence) = self.submission_order.front().copied() {
            let Some(message) = self.terminal.remove(&sequence) else {
                break;
            };
            self.submission_order.pop_front();
            self.cancelled.remove(&sequence);
            self.delivered.push_back(message);
        }
    }

    fn dispatch_ready_workers(&mut self) -> Vec<WorkerHostCommand> {
        let mut commands = Vec::new();
        for (worker_index, worker) in self.workers.iter_mut().enumerate() {
            if !worker.ready || worker.failed || worker.running.is_some() {
                continue;
            }
            let Some(request) = self.pending.pop_front() else {
                break;
            };
            worker.running = Some(request.sequence);
            commands.push(WorkerHostCommand::Run {
                worker_index,
                request,
            });
        }
        commands
    }

    fn fail_pending_if_unavailable(&mut self) {
        if self.workers.iter().all(|worker| worker.failed) {
            while let Some(request) = self.pending.pop_front() {
                let sequence = request.sequence;
                self.terminal.insert(
                    sequence,
                    WorkerMessage::Failed {
                        sequence,
                        error: WorkerFailure::Unavailable,
                    },
                );
            }
            self.release_ordered();
        }
    }
}

fn terminal_sequence(message: &WorkerMessage) -> u64 {
    match message {
        WorkerMessage::Complete { sequence, .. } | WorkerMessage::Failed { sequence, .. } => {
            *sequence
        }
        _ => unreachable!("only terminal messages enter the ordered completion buffer"),
    }
}

#[cfg(test)]
mod worker_operation_queue_tests {
    use super::super::work_executor::{
        CooperativeWorkerOperationExecutor, WorkerFailure, WorkerKernelRegistry, WorkerMessage,
        WorkerOperation, WorkerOperationExecutor, WorkerRequest,
    };
    use super::{WorkerHostCommand, WorkerOperationQueue, WorkerQueueError};

    fn operation() -> WorkerOperation {
        WorkerOperation::new("org.example.operation/v1").unwrap()
    }

    fn request(sequence: u64) -> WorkerRequest {
        WorkerRequest {
            sequence,
            operation: operation(),
            payload: vec![sequence as u8],
        }
    }

    fn run_sequence(command: &WorkerHostCommand) -> u64 {
        match command {
            WorkerHostCommand::Run { request, .. } => request.sequence,
            WorkerHostCommand::Cancel { .. } => panic!("expected a run command"),
        }
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn rejects_invalid_capacity_and_unregistered_or_non_monotonic_requests() {
        assert!(matches!(
            WorkerOperationQueue::new(0, 1, [operation()]),
            Err(WorkerQueueError::EmptyPool)
        ));
        assert!(matches!(
            WorkerOperationQueue::new(2, 1, [operation()]),
            Err(WorkerQueueError::InsufficientCapacity {
                worker_count: 2,
                max_outstanding: 1,
            })
        ));
        let mut queue = WorkerOperationQueue::new(1, 2, [operation()]).unwrap();
        let unknown = WorkerOperation::new("org.example.unknown/v1").unwrap();
        assert_eq!(
            queue
                .submit(WorkerRequest {
                    sequence: 1,
                    operation: unknown,
                    payload: Vec::new(),
                })
                .unwrap_err(),
            WorkerQueueError::OperationNotRegistered {
                operation: "org.example.unknown/v1".to_string(),
            }
        );
        queue.submit(request(2)).unwrap();
        assert_eq!(
            queue.submit(request(2)).unwrap_err(),
            WorkerQueueError::NonMonotonicSequence {
                sequence: 2,
                previous: 2,
            }
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn bounds_accepted_work_until_an_ordered_result_is_available() {
        let mut queue = WorkerOperationQueue::new(1, 2, [operation()]).unwrap();
        queue.worker_ready(0);
        assert_eq!(queue.submit(request(1)).unwrap().len(), 1);
        assert!(queue.submit(request(2)).unwrap().is_empty());
        assert_eq!(
            queue.submit(request(3)).unwrap_err(),
            WorkerQueueError::Full
        );
        let commands = queue.worker_completed(0, Some(1), Ok(vec![11]));
        assert_eq!(commands.len(), 1);
        assert_eq!(run_sequence(&commands[0]), 2);
        assert_eq!(queue.outstanding(), 1);
        assert!(queue.submit(request(3)).is_ok());
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn releases_terminal_messages_in_submission_order() {
        let mut queue = WorkerOperationQueue::new(2, 4, [operation()]).unwrap();
        queue.worker_ready(0);
        queue.worker_ready(1);
        let first = queue.submit(request(10)).unwrap();
        let second = queue.submit(request(11)).unwrap();
        assert_eq!(run_sequence(&first[0]), 10);
        assert_eq!(run_sequence(&second[0]), 11);

        queue.worker_completed(1, Some(11), Ok(vec![2]));
        assert!(queue.drain_messages().is_empty());
        queue.worker_completed(0, Some(10), Ok(vec![1]));
        assert_eq!(
            queue.drain_messages(),
            vec![
                WorkerMessage::Complete {
                    sequence: 10,
                    payload: vec![1],
                },
                WorkerMessage::Complete {
                    sequence: 11,
                    payload: vec![2],
                },
            ]
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn cancellation_covers_queued_and_active_requests_without_late_results() {
        let mut queue = WorkerOperationQueue::new(1, 3, [operation()]).unwrap();
        queue.worker_ready(0);
        queue.submit(request(1)).unwrap();
        queue.submit(request(2)).unwrap();

        let (accepted, commands) = queue.cancel(2);
        assert!(accepted);
        assert!(commands.is_empty());
        let (accepted, commands) = queue.cancel(1);
        assert!(accepted);
        assert_eq!(
            commands,
            vec![WorkerHostCommand::Cancel {
                worker_index: 0,
                sequence: 1,
            }]
        );
        assert_eq!(
            queue.drain_messages(),
            vec![
                WorkerMessage::Failed {
                    sequence: 1,
                    error: WorkerFailure::Cancelled,
                },
                WorkerMessage::Failed {
                    sequence: 2,
                    error: WorkerFailure::Cancelled,
                },
            ]
        );
        queue.worker_progress(1, 1, Some(1));
        queue.worker_completed(0, Some(1), Ok(vec![99]));
        assert!(queue.drain_messages().is_empty());
        assert!(!queue.cancel(1).0);
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn worker_failure_preserves_order_and_fails_pending_work_when_pool_is_lost() {
        let mut queue = WorkerOperationQueue::new(2, 4, [operation()]).unwrap();
        queue.worker_ready(0);
        queue.worker_ready(1);
        queue.submit(request(1)).unwrap();
        queue.submit(request(2)).unwrap();
        queue.submit(request(3)).unwrap();

        queue.worker_completed(1, Some(2), Ok(vec![2]));
        queue.worker_failed(
            0,
            WorkerFailure::Host {
                message: "worker zero failed".to_string(),
            },
        );
        assert_eq!(
            queue.drain_messages(),
            vec![
                WorkerMessage::Failed {
                    sequence: 1,
                    error: WorkerFailure::Host {
                        message: "worker zero failed".to_string(),
                    },
                },
                WorkerMessage::Complete {
                    sequence: 2,
                    payload: vec![2],
                },
            ]
        );
        queue.worker_failed(
            1,
            WorkerFailure::Host {
                message: "worker one failed".to_string(),
            },
        );
        assert_eq!(
            queue.drain_messages(),
            vec![WorkerMessage::Failed {
                sequence: 3,
                error: WorkerFailure::Host {
                    message: "worker one failed".to_string(),
                },
            }]
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn mismatched_completion_fails_the_actual_request() {
        let mut queue = WorkerOperationQueue::new(1, 1, [operation()]).unwrap();
        queue.worker_ready(0);
        queue.submit(request(7)).unwrap();
        queue.worker_completed(0, Some(8), Ok(vec![8]));
        assert_eq!(
            queue.drain_messages(),
            vec![WorkerMessage::Failed {
                sequence: 7,
                error: WorkerFailure::Protocol {
                    message: "worker returned sequence 8 while running sequence 7".to_string(),
                },
            }]
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn parallel_queue_and_cooperative_fallback_emit_equivalent_results() {
        let mut kernels = WorkerKernelRegistry::new();
        kernels
            .register(operation().as_str(), |mut payload| {
                payload.reverse();
                Ok(payload)
            })
            .unwrap();
        let cooperative =
            CooperativeWorkerOperationExecutor::new(kernels.clone(), "parallel host unavailable");
        let mut queue = WorkerOperationQueue::new(1, 2, [operation()]).unwrap();
        queue.worker_ready(0);
        let request = WorkerRequest {
            sequence: 42,
            operation: operation(),
            payload: vec![1, 2, 3],
        };

        cooperative.submit(request.clone()).unwrap();
        let commands = queue.submit(request).unwrap();
        let WorkerHostCommand::Run {
            worker_index,
            request,
        } = commands.into_iter().next().unwrap()
        else {
            panic!("expected a run command");
        };
        queue.worker_progress(request.sequence, 0, Some(1));
        let result = match kernels.execute(request.clone()) {
            WorkerMessage::Complete { payload, .. } => Ok(payload),
            WorkerMessage::Failed { error, .. } => Err(error),
            _ => panic!("kernel returned a non-terminal message"),
        };
        queue.worker_progress(request.sequence, 1, Some(1));
        queue.worker_completed(worker_index, Some(request.sequence), result);

        assert_eq!(queue.drain_messages(), cooperative.drain_messages());
    }
}
