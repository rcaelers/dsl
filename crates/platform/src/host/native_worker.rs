use std::sync::Mutex;

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};

use platform_runtime::{
    WorkerExecutionCapability, WorkerFailure, WorkerHostCommand, WorkerKernelRegistry,
    WorkerMessage, WorkerOperationExecutor, WorkerOperationQueue, WorkerQueueError, WorkerRequest,
};

enum WorkerEvent {
    Progress {
        sequence: u64,
        completed: u64,
        total: Option<u64>,
    },
    Complete {
        worker_index: usize,
        sequence: u64,
        result: Result<Vec<u8>, WorkerFailure>,
    },
}

struct AdapterState {
    queue: WorkerOperationQueue,
    workers: Vec<Sender<WorkerRequest>>,
    events: Receiver<WorkerEvent>,
}

/// Native transport for the portable finite-operation queue.
pub(crate) struct NativeWorkerOperationExecutor {
    state: Mutex<AdapterState>,
}

impl NativeWorkerOperationExecutor {
    pub(crate) fn new(kernels: WorkerKernelRegistry) -> Result<Self, String> {
        let worker_count = native_worker_count();
        Self::with_registry(kernels, worker_count, worker_count.saturating_mul(4))
    }

    fn with_registry(
        kernels: WorkerKernelRegistry,
        worker_count: usize,
        max_outstanding: usize,
    ) -> Result<Self, String> {
        let operations = kernels.operations().cloned().collect::<Vec<_>>();
        let mut queue = WorkerOperationQueue::new(worker_count, max_outstanding, operations)
            .map_err(|error| error.to_string())?;
        let (event_sender, events) = crossbeam_channel::unbounded();
        let mut workers = Vec::with_capacity(worker_count);

        for worker_index in 0..worker_count {
            let (sender, receiver) = crossbeam_channel::bounded(1);
            let worker_events = event_sender.clone();
            let worker_kernels = kernels.clone();
            std::thread::Builder::new()
                .name(format!("portable-operation-{worker_index}"))
                .spawn(move || run_worker(worker_index, receiver, worker_events, worker_kernels))
                .map_err(|error| format!("could not start native operation worker: {error}"))?;
            workers.push(sender);
            let commands = queue.worker_ready(worker_index);
            debug_assert!(commands.is_empty());
        }

        Ok(Self {
            state: Mutex::new(AdapterState {
                queue,
                workers,
                events,
            }),
        })
    }
}

impl WorkerOperationExecutor for NativeWorkerOperationExecutor {
    fn capability(&self) -> WorkerExecutionCapability {
        let mut state = self.state.lock().unwrap();
        pump_events(&mut state);
        state.queue.capability()
    }

    fn submit(&self, request: WorkerRequest) -> Result<(), WorkerQueueError> {
        let mut state = self.state.lock().unwrap();
        pump_events(&mut state);
        let commands = state.queue.submit(request)?;
        apply_commands(&mut state, commands);
        Ok(())
    }

    fn cancel(&self, sequence: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        pump_events(&mut state);
        let (accepted, commands) = state.queue.cancel(sequence);
        apply_commands(&mut state, commands);
        accepted
    }

    fn drain_messages(&self) -> Vec<WorkerMessage> {
        let mut state = self.state.lock().unwrap();
        pump_events(&mut state);
        state.queue.drain_messages()
    }

    fn outstanding(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        pump_events(&mut state);
        state.queue.outstanding()
    }
}

fn native_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(2, 32)
}

fn run_worker(
    worker_index: usize,
    requests: Receiver<WorkerRequest>,
    events: Sender<WorkerEvent>,
    kernels: WorkerKernelRegistry,
) {
    while let Ok(request) = requests.recv() {
        let sequence = request.sequence;
        if events
            .send(WorkerEvent::Progress {
                sequence,
                completed: 0,
                total: Some(1),
            })
            .is_err()
        {
            return;
        }
        let terminal =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| kernels.execute(request)));
        let result = match terminal {
            Ok(WorkerMessage::Complete { payload, .. }) => Ok(payload),
            Ok(WorkerMessage::Failed { error, .. }) => Err(error),
            Ok(_) => Err(WorkerFailure::Protocol {
                message: "worker kernel returned a non-terminal message".to_string(),
            }),
            Err(_) => Err(WorkerFailure::Kernel {
                message: "worker operation panicked".to_string(),
            }),
        };
        if events
            .send(WorkerEvent::Progress {
                sequence,
                completed: 1,
                total: Some(1),
            })
            .is_err()
        {
            return;
        }
        if events
            .send(WorkerEvent::Complete {
                worker_index,
                sequence,
                result,
            })
            .is_err()
        {
            return;
        }
    }
}

fn pump_events(state: &mut AdapterState) {
    loop {
        match state.events.try_recv() {
            Ok(WorkerEvent::Progress {
                sequence,
                completed,
                total,
            }) => state.queue.worker_progress(sequence, completed, total),
            Ok(WorkerEvent::Complete {
                worker_index,
                sequence,
                result,
            }) => {
                let commands = state
                    .queue
                    .worker_completed(worker_index, Some(sequence), result);
                apply_commands(state, commands);
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
        }
    }
}

fn apply_commands(state: &mut AdapterState, mut commands: Vec<WorkerHostCommand>) {
    while let Some(command) = commands.pop() {
        match command {
            WorkerHostCommand::Run {
                worker_index,
                request,
            } => {
                let result = state.workers[worker_index].try_send(request);
                if let Err(error) = result {
                    let message = match error {
                        TrySendError::Full(_) => {
                            "native operation worker transport is unexpectedly full".to_string()
                        }
                        TrySendError::Disconnected(_) => {
                            "native operation worker stopped".to_string()
                        }
                    };
                    commands.extend(
                        state
                            .queue
                            .worker_failed(worker_index, WorkerFailure::Host { message }),
                    );
                }
            }
            WorkerHostCommand::Cancel { .. } => {
                // Portable kernels are finite and synchronous. The queue
                // publishes cancellation immediately and suppresses the late
                // completion without claiming to preempt native code.
            }
        }
    }
}

#[cfg(test)]
mod native_worker_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use platform_runtime::{
        WorkerExecutionMode, WorkerFailure, WorkerKernelRegistry, WorkerMessage, WorkerOperation,
        WorkerOperationExecutor, WorkerRequest,
    };

    use super::NativeWorkerOperationExecutor;

    fn operation() -> WorkerOperation {
        WorkerOperation::new("org.example.test.reverse/v1").unwrap()
    }

    fn request(sequence: u64, payload: Vec<u8>) -> WorkerRequest {
        WorkerRequest {
            sequence,
            operation: operation(),
            payload,
        }
    }

    fn wait_for_messages(
        executor: &NativeWorkerOperationExecutor,
        terminal_count: usize,
    ) -> Vec<WorkerMessage> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut messages = Vec::new();
        while messages
            .iter()
            .filter(|message| {
                matches!(
                    message,
                    WorkerMessage::Complete { .. } | WorkerMessage::Failed { .. }
                )
            })
            .count()
            < terminal_count
        {
            messages.extend(executor.drain_messages());
            assert!(Instant::now() < deadline, "native workers did not finish");
            std::thread::yield_now();
        }
        messages
    }

    #[test]
    fn native_operations_report_parallel_capability_and_ordered_results() {
        let mut kernels = WorkerKernelRegistry::new();
        kernels
            .register(operation().as_str(), |mut payload| {
                payload.reverse();
                Ok(payload)
            })
            .unwrap();
        let executor = NativeWorkerOperationExecutor::with_registry(kernels, 2, 4).unwrap();

        assert_eq!(executor.capability().mode(), WorkerExecutionMode::Parallel);
        assert_eq!(executor.capability().parallelism(), 2);
        executor.submit(request(10, vec![1, 2])).unwrap();
        executor.submit(request(11, vec![3, 4])).unwrap();

        let terminal = wait_for_messages(&executor, 2)
            .into_iter()
            .filter(|message| {
                matches!(
                    message,
                    WorkerMessage::Complete { .. } | WorkerMessage::Failed { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminal,
            vec![
                WorkerMessage::Complete {
                    sequence: 10,
                    payload: vec![2, 1],
                },
                WorkerMessage::Complete {
                    sequence: 11,
                    payload: vec![4, 3],
                },
            ]
        );
    }

    #[test]
    fn native_operations_bound_work_and_suppress_cancelled_results() {
        let release = Arc::new(AtomicBool::new(false));
        let kernel_release = Arc::clone(&release);
        let mut kernels = WorkerKernelRegistry::new();
        kernels
            .register(operation().as_str(), move |payload| {
                while !kernel_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                Ok(payload)
            })
            .unwrap();
        let executor = NativeWorkerOperationExecutor::with_registry(kernels, 1, 1).unwrap();

        executor.submit(request(1, vec![1])).unwrap();
        assert!(executor.submit(request(2, vec![2])).is_err());
        assert!(executor.cancel(1));
        assert_eq!(
            executor
                .drain_messages()
                .into_iter()
                .filter(|message| matches!(message, WorkerMessage::Failed { .. }))
                .collect::<Vec<_>>(),
            vec![WorkerMessage::Failed {
                sequence: 1,
                error: WorkerFailure::Cancelled,
            }]
        );

        release.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(2);
        while executor.outstanding() != 0 {
            assert!(Instant::now() < deadline, "cancelled worker did not finish");
            std::thread::yield_now();
        }
        assert!(
            executor
                .drain_messages()
                .into_iter()
                .all(|message| { !matches!(message, WorkerMessage::Complete { sequence: 1, .. }) })
        );
    }

    #[test]
    fn native_operation_panics_become_ordered_failures() {
        let mut kernels = WorkerKernelRegistry::new();
        kernels
            .register(operation().as_str(), |_| panic!("kernel panic"))
            .unwrap();
        let executor = NativeWorkerOperationExecutor::with_registry(kernels, 1, 1).unwrap();

        executor.submit(request(7, Vec::new())).unwrap();
        let terminal = wait_for_messages(&executor, 1)
            .into_iter()
            .find(|message| matches!(message, WorkerMessage::Failed { .. }))
            .unwrap();
        assert_eq!(
            terminal,
            WorkerMessage::Failed {
                sequence: 7,
                error: WorkerFailure::Kernel {
                    message: "worker operation panicked".to_string(),
                },
            }
        );
    }
}
