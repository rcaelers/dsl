use super::PreparedCaptureData;

/// Result produced when a source-preparation task completes.
pub type SourcePreparationResult = Result<PreparedCaptureData, String>;

/// One source-preparation operation accepted by a host executor.
pub type SourcePreparationWork = Box<dyn FnOnce() -> SourcePreparationResult + Send + 'static>;

/// The current state of one submitted source-preparation operation.
pub enum SourcePreparationTaskUpdate {
    Pending,
    Complete(SourcePreparationResult),
    Disconnected,
}

/// A host-owned source-preparation operation that the compiler can poll.
pub trait SourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate;
}

/// Host execution contract for finite-source preparation.
///
/// The compiler owns the task data model and polls the returned task. Hosts
/// decide whether work runs inline, on native workers, or in a future browser
/// worker without introducing target selection into compiler behavior.
pub trait SourcePreparationExecutor: Send + Sync {
    fn submit(&self, work: SourcePreparationWork)
    -> Result<Box<dyn SourcePreparationTask>, String>;
}

/// Portable executor for hosts that advance preparation inline.
pub struct InlineSourcePreparationExecutor;

impl SourcePreparationExecutor for InlineSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        Ok(Box::new(InlineSourcePreparationTask {
            result: Some(work()),
        }))
    }
}

struct InlineSourcePreparationTask {
    result: Option<SourcePreparationResult>,
}

impl SourcePreparationTask for InlineSourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate {
        self.result
            .take()
            .map(SourcePreparationTaskUpdate::Complete)
            .unwrap_or(SourcePreparationTaskUpdate::Disconnected)
    }
}
