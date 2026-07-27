use super::PreparedCaptureData;

pub(crate) type SourcePreparationResult = Result<PreparedCaptureData, String>;
pub(crate) type SourcePreparationWork =
    Box<dyn FnOnce() -> SourcePreparationResult + Send + 'static>;

pub(crate) enum SourcePreparationTaskUpdate {
    Pending,
    Complete(SourcePreparationResult),
    Disconnected,
}

pub(crate) trait SourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate;
}

pub(crate) trait SourcePreparationExecutor {
    fn submit(&self, work: SourcePreparationWork)
    -> Result<Box<dyn SourcePreparationTask>, String>;
}
