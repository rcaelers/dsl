//! Host-executed scheduler for streaming graphs.
//!
//! Submits one long-lived task per node through a host-selected executor and
//! manages their lifecycle through completion handles.
//!
//! ## Execution Models
//!
//! The scheduler supports two threading models:
//!
//! 1. **Regular nodes**: Scheduler calls `work()` repeatedly in a loop. The node processes
//!    one batch of items per call and returns the count. The scheduled task does the looping.
//!
//! 2. **Self-executing nodes**: Node manages its own long-lived operation. Scheduler calls
//!    `work()` once to start the node, then waits for `should_stop()` to signal completion.
//!    The node returns `is_self_threading() = true` to indicate this pattern.
//!
//! A self-executing source can, for example, arrange one reader per output destination.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, error, info};

use platform_runtime::{WorkExecutor, WorkTask};

use super::node::ProcessNode;
use super::ports::{InputPort, OutputPort};
use super::watchdog::Watchdog;

/// Threaded runtime owner for a statically built streaming graph.
///
/// The scheduler owns node tasks and the watchdog task. Dropping it requests
/// stop, while [`Self::wait`] explicitly joins all outstanding work.
pub struct Scheduler {
    tasks: Vec<(String, Box<dyn WorkTask>)>,
    stop_signal: Arc<AtomicBool>,
    watchdog: Watchdog,
    watchdog_task: Option<Box<dyn WorkTask>>,
    work_executor: Arc<dyn WorkExecutor>,
}

impl Scheduler {
    /// Creates a scheduler and starts its watchdog monitor.
    ///
    /// # Parameters
    /// - `work_executor`: Host capability used for node and watchdog tasks.
    pub fn new(work_executor: Arc<dyn WorkExecutor>) -> Self {
        let watchdog = Watchdog::new();
        let watchdog_task = watchdog
            .start_monitoring(Arc::clone(&work_executor))
            .expect("host work executor must accept watchdog monitoring");
        info!("Watchdog enabled - will report operations blocked >5 seconds");
        Self {
            tasks: Vec::new(),
            stop_signal: Arc::new(AtomicBool::new(false)),
            watchdog,
            watchdog_task: Some(watchdog_task),
            work_executor,
        }
    }

    /// Returns the watchdog used to register pipeline port operations.
    pub fn watchdog(&self) -> &Watchdog {
        &self.watchdog
    }

    /// Starts one process node with its resolved input and output ports.
    ///
    /// Nodes may be sources (zero inputs), sinks (zero outputs), or
    /// transforms. The scheduler owns the submitted task until [`Self::wait`].
    ///
    /// # Parameters
    ///
    /// - `node`: Process implementation to run.
    /// - `inputs`: Transport/query capabilities arranged in input-schema order.
    /// - `outputs`: Transport capabilities arranged in output-schema order.
    pub fn start_process(
        &mut self,
        mut node: Box<dyn ProcessNode>,
        inputs: Vec<InputPort>,
        outputs: Vec<OutputPort>,
    ) {
        let stop_signal = Arc::clone(&self.stop_signal);
        let work_executor = Arc::clone(&self.work_executor);
        let name = node.name().to_string();
        let thread_name = name.clone();

        debug!("Starting process node: {}", name);

        let task_executor = Arc::clone(&work_executor);
        let task = work_executor
            .submit_long_running(Box::new(move || {
                if node.is_self_threading() {
                    // Self-threading node: call work() once to start internal threads
                    if let Err(e) = node.work_outcome(&inputs, &outputs) {
                        error!(
                            "[{}] Failed to start self-threading node: {}",
                            thread_name, e
                        );
                    } else {
                        // Wait for node to signal completion via should_stop() or stop_signal
                        loop {
                            if stop_signal.load(Ordering::Relaxed) {
                                info!(
                                    "[{}] Stop signal received, shutting down self-threading node",
                                    thread_name
                                );
                                break;
                            }
                            if node.should_stop() {
                                info!("[{}] Self-threading node completed", thread_name);
                                break;
                            }
                            task_executor.idle(std::time::Duration::from_millis(100));
                        }
                    }

                    // Drop outputs/inputs/node to trigger shutdown
                    drop(outputs);
                    drop(inputs);
                    drop(node);
                } else {
                    // Regular node: call work() repeatedly
                    let mut items_produced = 0usize;

                    loop {
                        if stop_signal.load(Ordering::Relaxed) || node.should_stop() {
                            break;
                        }

                        match node.work_outcome(&inputs, &outputs) {
                            Ok(outcome) => {
                                items_produced += outcome.produced_items();
                                if outcome.produced_items() == 0 {
                                    task_executor.idle(std::time::Duration::from_millis(2));
                                }
                            }
                            Err(e) => {
                                error!("[{}] Work error: {}", thread_name, e);
                                break;
                            }
                        }
                    }

                    info!(
                        "[{}] Shutdown. Produced {} items.",
                        thread_name, items_produced
                    );

                    // Drop outputs/inputs/node to close channels
                    drop(outputs);
                    drop(inputs);
                    drop(node);
                }
            }))
            .expect("host work executor must accept process work");

        self.tasks.push((name, task));
    }

    /// Requests cooperative cancellation from all running node loops.
    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::Relaxed);
    }

    /// Returns a handle that can request stop while [`Self::wait`]
    /// owns the scheduler (e.g. a UI Stop button).
    pub fn stop_handle(&self) -> StopHandle {
        StopHandle(Arc::clone(&self.stop_signal))
    }

    /// Stops the watchdog and blocks until every node task completes.
    pub fn wait(mut self) {
        let total_tasks = self.tasks.len();
        info!("Waiting for {} tasks to complete...", total_tasks);
        for (name, task) in self.tasks.drain(..) {
            task.wait();
            info!("[{name}] completed");
        }
        info!("All {} tasks completed", total_tasks);

        // Stop watchdog
        self.watchdog.stop();
        if let Some(task) = self.watchdog_task.take() {
            task.wait();
        }
    }

    /// Returns the number of node tasks currently owned by the scheduler.
    pub fn num_threads(&self) -> usize {
        self.tasks.len()
    }

    /// Returns node names for tasks currently owned by the scheduler.
    pub fn thread_names(&self) -> Vec<String> {
        self.tasks.iter().map(|(name, _)| name.clone()).collect()
    }
}

/// Cloneable stop signal detached from the scheduler's ownership.
#[derive(Clone)]
pub struct StopHandle(Arc<AtomicBool>);

impl StopHandle {
    /// Requests cooperative cancellation from all nodes in the scheduler.
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

// /// Helper to create channels for a connection
// pub fn create_channel<T: Send>(buffer_size: usize) -> (Sender<T>, Receiver<T>) {
//     bounded(buffer_size)
// }

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use crossbeam_channel::bounded;

    use platform_runtime::{WorkExecutor, WorkExecutorError, WorkExecutorTask, WorkTask};

    use super::super::errors::{WorkError, WorkResult};
    use super::super::node::ProcessNode;
    use super::super::sender::ChannelMessage;
    use super::*;

    struct TestWorkExecutor;

    impl WorkExecutor for TestWorkExecutor {
        fn available_parallelism(&self) -> usize {
            2
        }

        fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
            Ok(Box::new(TestWorkTask {
                handle: Some(thread::spawn(task)),
            }))
        }
    }

    struct TestWorkTask {
        handle: Option<JoinHandle<()>>,
    }

    impl WorkTask for TestWorkTask {
        fn is_finished(&self) -> bool {
            self.handle.as_ref().is_none_or(JoinHandle::is_finished)
        }

        fn wait(mut self: Box<Self>) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn work_executor() -> Arc<dyn WorkExecutor> {
        Arc::new(TestWorkExecutor)
    }

    struct TestSource {
        count: usize,
        max: usize,
    }

    impl ProcessNode for TestSource {
        fn name(&self) -> &str {
            "test_source"
        }

        fn should_stop(&self) -> bool {
            self.count >= self.max
        }

        fn num_inputs(&self) -> usize {
            0 // Source
        }

        fn num_outputs(&self) -> usize {
            1
        }

        fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
            let output = outputs[0]
                .get::<u32>()
                .ok_or_else(|| WorkError::NodeError("Missing output channel".to_string()))?;

            if self.count < self.max {
                output.send(self.count as u32)?;
                self.count += 1;
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }

    struct TestSink {
        received: Arc<Mutex<Vec<u32>>>,
    }

    impl ProcessNode for TestSink {
        fn name(&self) -> &str {
            "test_sink"
        }

        fn num_inputs(&self) -> usize {
            1
        }

        fn num_outputs(&self) -> usize {
            0 // Sink
        }

        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let mut input_buffer = std::collections::VecDeque::new();
            let mut input = inputs[0]
                .get::<u32>(&mut input_buffer)
                .ok_or_else(|| WorkError::NodeError("Missing input channel".to_string()))?;

            match input.recv_timeout(Duration::from_millis(100)) {
                Ok(value) => {
                    self.received.lock().unwrap().push(value);
                    Ok(1)
                }
                Err(_) => {
                    tracing::debug!("[TestSink] recv_timeout error, returning Shutdown");
                    Err(WorkError::Shutdown)
                }
            }
        }
    }

    #[test]
    fn test_scheduler_basic() {
        let mut scheduler = Scheduler::new(work_executor());

        let (tx, rx) = bounded::<ChannelMessage<u32>>(10);

        let source = TestSource { count: 0, max: 5 };
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = TestSink {
            received: Arc::clone(&received),
        };

        // Create test watchdog
        let watchdog = super::super::watchdog::Watchdog::new();

        // Source has 0 inputs, 1 output
        let source_outputs = vec![OutputPort::new_with_watchdog(
            super::super::sender::Sender::new(vec![tx]),
            &watchdog,
            "test_source",
            "output",
        )];
        scheduler.start_process(Box::new(source), vec![], source_outputs);

        // Sink has 1 input, 0 outputs
        let sink_inputs = vec![InputPort::new_with_watchdog(
            rx,
            &watchdog,
            "test_sink",
            "input",
        )];
        scheduler.start_process(Box::new(sink), sink_inputs, vec![]);

        thread::sleep(Duration::from_millis(200));

        let values = received.lock().unwrap();
        assert_eq!(*values, vec![0, 1, 2, 3, 4]);
    }

    // Self-threading test node that runs until stopped
    struct SelfThreadingTestNode {
        stop: Arc<AtomicBool>,
        completed: Arc<AtomicBool>,
    }

    impl ProcessNode for SelfThreadingTestNode {
        fn name(&self) -> &str {
            "self_threading_test"
        }

        fn is_self_threading(&self) -> bool {
            true
        }

        fn should_stop(&self) -> bool {
            self.completed.load(Ordering::Relaxed)
        }

        fn num_inputs(&self) -> usize {
            0
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let stop = Arc::clone(&self.stop);
            let completed = Arc::clone(&self.completed);

            // Spawn internal worker thread
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(10));
                }
                completed.store(true, Ordering::Relaxed);
            });

            Ok(0)
        }
    }

    impl Drop for SelfThreadingTestNode {
        fn drop(&mut self) {
            // Signal thread to stop
            self.stop.store(true, Ordering::Relaxed);
            // Wait for completion (with timeout to avoid hanging test)
            for _ in 0..100 {
                if self.completed.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn test_scheduler_stop_signal_self_threading() {
        let mut scheduler = Scheduler::new(work_executor());

        let stop = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));

        let node = SelfThreadingTestNode {
            stop: Arc::clone(&stop),
            completed: Arc::clone(&completed),
        };

        scheduler.start_process(Box::new(node), vec![], vec![]);

        // Wait a bit to ensure thread starts
        thread::sleep(Duration::from_millis(50));

        // Signal stop
        scheduler.stop();

        // Wait for completion (this should happen quickly)
        let start = std::time::Instant::now();
        scheduler.wait();
        let elapsed = start.elapsed();

        // Should complete within a reasonable time (not hang forever)
        assert!(
            elapsed < Duration::from_secs(2),
            "Scheduler took too long to stop: {:?}",
            elapsed
        );

        // Verify the node's thread was stopped
        assert!(
            completed.load(Ordering::Relaxed),
            "Self-threading node did not complete"
        );
    }
}
