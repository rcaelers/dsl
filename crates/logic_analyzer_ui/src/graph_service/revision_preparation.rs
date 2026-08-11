use std::sync::{Arc, Mutex};

use logic_analyzer_graph_plan::{ProcessingGraph, ProcessingGraphError};
use platform_runtime::{WorkExecutor, WorkExecutorError, WorkTask};

type RevisionPreparation =
    Box<dyn FnOnce() -> Result<ProcessingGraph, Vec<ProcessingGraphError>> + Send>;

/// Immutable graph revision lowered away from the application frame path.
pub(crate) struct PreparedGraphRevision {
    pub(crate) revision: u64,
    pub(crate) processing_graph: Result<ProcessingGraph, Vec<ProcessingGraphError>>,
}

/// Completion handle for one revision-tagged graph-lowering request.
pub(crate) struct GraphRevisionPreparationTask {
    completion: Arc<Mutex<Option<PreparedGraphRevision>>>,
    task: Option<Box<dyn WorkTask>>,
}

impl GraphRevisionPreparationTask {
    pub(crate) fn start(
        revision: u64,
        executor: Arc<dyn WorkExecutor>,
        prepare: RevisionPreparation,
    ) -> Result<Self, WorkExecutorError> {
        let completion = Arc::new(Mutex::new(None));
        let task_completion = Arc::clone(&completion);
        let task = executor.submit_labeled(
            "graph revision preparation",
            Box::new(move || {
                let processing_graph = prepare();
                *task_completion.lock().unwrap() = Some(PreparedGraphRevision {
                    revision,
                    processing_graph,
                });
            }),
        )?;
        Ok(Self {
            completion,
            task: Some(task),
        })
    }

    pub(crate) fn poll(&mut self) -> Option<PreparedGraphRevision> {
        if !self.task.as_ref().is_some_and(|task| task.is_finished()) {
            return None;
        }
        self.task.take().expect("finished task exists").wait();
        Some(
            self.completion
                .lock()
                .unwrap()
                .take()
                .expect("finished graph preparation published its result"),
        )
    }
}

#[cfg(test)]
mod revision_preparation_tests {
    use platform_runtime::InlineWorkExecutor;

    use super::*;

    #[test]
    fn completion_retains_the_submitted_revision() {
        let mut task = GraphRevisionPreparationTask::start(
            42,
            Arc::new(InlineWorkExecutor),
            Box::new(|| Err(Vec::new())),
        )
        .unwrap();

        let prepared = task.poll().expect("inline lowering completes immediately");
        assert_eq!(prepared.revision, 42);
    }
}
