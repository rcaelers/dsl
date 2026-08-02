use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use signal_processing::CaptureIndexBuildProgress;

use super::PreparedCaptureData;

/// Result produced when a source-preparation task completes.
pub type SourcePreparationResult = Result<PreparedCaptureData, String>;

/// One source-preparation operation accepted by a host executor.
pub type SourcePreparationWork =
    Box<dyn FnOnce(SourcePreparationControl) -> SourcePreparationResult + Send + 'static>;

/// Shared progress and cancellation state for one preparation generation.
#[derive(Clone)]
pub struct SourcePreparationControl {
    inner: Arc<SourcePreparationControlState>,
}

struct SourcePreparationControlState {
    cancelled: AtomicBool,
    progress: Mutex<Option<CaptureIndexBuildProgress>>,
}

impl SourcePreparationControl {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SourcePreparationControlState {
                cancelled: AtomicBool::new(false),
                progress: Mutex::new(None),
            }),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn report_progress(&self, progress: CaptureIndexBuildProgress) -> bool {
        if self.is_cancelled() {
            return false;
        }
        *self.inner.progress.lock().unwrap() = Some(progress);
        true
    }

    pub(crate) fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn progress(&self) -> Option<CaptureIndexBuildProgress> {
        *self.inner.progress.lock().unwrap()
    }
}

impl Default for SourcePreparationControl {
    fn default() -> Self {
        Self::new()
    }
}

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
    fn submit(
        &self,
        work: SourcePreparationWork,
        control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String>;
}

/// Portable executor for hosts that advance preparation inline.
pub struct InlineSourcePreparationExecutor;

impl SourcePreparationExecutor for InlineSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
        control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        Ok(Box::new(InlineSourcePreparationTask {
            result: Some(work(control)),
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
