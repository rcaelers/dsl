/// One bounded unit of host-scheduled work.
pub type WorkExecutorTask = Box<dyn FnOnce() + Send + 'static>;

/// Host execution capability used by portable processing nodes.
pub trait WorkExecutor: Send + Sync {
    /// Number of independent tasks the host can run concurrently.
    fn available_parallelism(&self) -> usize;

    /// Enqueues one task without exposing worker implementation details.
    fn submit(&self, task: WorkExecutorTask) -> Result<(), String>;
}

/// Portable executor that performs work on the calling cooperative pump.
pub struct InlineWorkExecutor;

impl WorkExecutor for InlineWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<(), String> {
        task();
        Ok(())
    }
}
