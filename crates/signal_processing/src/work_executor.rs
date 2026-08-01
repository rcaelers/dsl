/// One bounded unit of host-scheduled work.
pub type WorkExecutorTask = Box<dyn FnOnce() + Send + 'static>;

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

    /// Enqueues one task without exposing worker implementation details.
    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String>;
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
