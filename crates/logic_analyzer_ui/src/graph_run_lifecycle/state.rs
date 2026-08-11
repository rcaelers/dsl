use logic_analyzer_graph_plan::SamplingOverlayCandidate;
use logic_analyzer_graph_runtime::DerivedCacheClearTask;
use node_graph::GraphState;

use crate::graph_service::{
    GraphRevisionPreparationTask, GraphRun, GraphRunFailure, PreparedGraphRevision, UiGraphService,
};

/// One bounded foreground-run polling outcome handed back to the application shell.
pub(crate) struct GraphRunPoll {
    pub(crate) failure: Option<GraphRunFailure>,
    pub(crate) synchronized: Result<bool, Vec<logic_analyzer_graph_plan::ProcessingGraphError>>,
    pub(crate) sampling_overlay_candidates: Option<Vec<SamplingOverlayCandidate>>,
    pub(crate) finished: bool,
}

/// Owns foreground graph execution and the state used to synchronize it.
///
/// A semantic baseline is installed with an active run and cleared when presentation data is
/// detached. The preview revision belongs to the idle cached-data path. Persistent run status is
/// kept here while one-off notifications remain owned by the application shell.
pub(crate) struct GraphRunLifecycle {
    graph_service: UiGraphService,
    run: Option<Box<dyn GraphRun>>,
    run_message: Option<(String, bool)>,
    running_graph_revision: Option<u64>,
    cached_preview_revision: Option<u64>,
    observed_document_revision: Option<u64>,
    last_semantic_edit: f64,
    last_submitted_revision: Option<u64>,
    revision_preparation: Option<GraphRevisionPreparationTask>,
    last_progress_update: f64,
    sampling_overlay_candidates: Vec<SamplingOverlayCandidate>,
    derived_cache_clear_task: Option<DerivedCacheClearTask>,
}

impl GraphRunLifecycle {
    pub(crate) fn new(graph_service: UiGraphService) -> Self {
        Self {
            graph_service,
            run: None,
            run_message: None,
            running_graph_revision: None,
            cached_preview_revision: None,
            observed_document_revision: None,
            last_semantic_edit: -1.0,
            last_submitted_revision: None,
            revision_preparation: None,
            last_progress_update: -1.0,
            sampling_overlay_candidates: Vec::new(),
            derived_cache_clear_task: None,
        }
    }

    pub(crate) fn service(&self) -> &UiGraphService {
        &self.graph_service
    }

    pub(crate) fn service_mut(&mut self) -> &mut UiGraphService {
        &mut self.graph_service
    }

    pub(crate) fn run(&self) -> Option<&dyn GraphRun> {
        self.run.as_deref()
    }

    pub(crate) fn take_run(&mut self) -> Option<Box<dyn GraphRun>> {
        self.running_graph_revision = None;
        self.run.take()
    }

    pub(crate) fn install_run(&mut self, run: Box<dyn GraphRun>, revision: u64) {
        self.run = Some(run);
        self.running_graph_revision = Some(revision);
    }

    pub(crate) fn has_run(&self) -> bool {
        self.run.is_some()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|run| !run.is_finished())
    }

    pub(crate) fn is_stopping(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.is_stopping())
    }

    pub(crate) fn stop_run(&mut self) {
        if let Some(run) = &mut self.run {
            run.stop();
        }
    }

    pub(crate) fn apply_prepared_run(
        &mut self,
        graph: logic_analyzer_graph_plan::ProcessingGraph,
    ) -> Option<
        Result<
            logic_analyzer_graph_runtime::ApplySummary,
            logic_analyzer_graph_runtime::ApplyError,
        >,
    > {
        let run = self.run.as_deref_mut()?;
        Some(run.apply_processing_graph(graph, None))
    }

    pub(crate) fn poll_run(&mut self, graph: &GraphState) -> Option<GraphRunPoll> {
        let run = self.run.as_deref_mut()?;
        run.pump_for(256, std::time::Duration::from_millis(8));
        let failure = run.take_failure();
        let synchronized = self.graph_service.synchronize_run_data(run, graph);
        let candidates = synchronized
            .as_ref()
            .is_ok_and(|changed| *changed)
            .then(|| run.sampling_overlays().to_vec());
        Some(GraphRunPoll {
            failure,
            synchronized,
            sampling_overlay_candidates: candidates,
            finished: run.is_finished(),
        })
    }

    pub(crate) fn run_is_finished_or_stopping(&self) -> bool {
        self.run
            .as_ref()
            .is_none_or(|run| run.is_finished() || run.is_stopping())
    }

    pub(crate) fn running_graph_revision(&self) -> Option<u64> {
        self.running_graph_revision
    }

    pub(crate) fn set_running_graph_revision(&mut self, revision: u64) {
        self.running_graph_revision = Some(revision);
    }

    pub(crate) fn run_message(&self) -> Option<&(String, bool)> {
        self.run_message.as_ref()
    }

    pub(crate) fn set_run_message(&mut self, message: impl Into<String>, is_error: bool) {
        self.run_message = Some((message.into(), is_error));
    }

    pub(crate) fn clear_run_message(&mut self) {
        self.run_message = None;
    }

    pub(crate) fn cached_preview_revision(&self) -> Option<u64> {
        self.cached_preview_revision
    }

    pub(crate) fn set_cached_preview_revision(&mut self, revision: u64) {
        self.cached_preview_revision = Some(revision);
    }

    pub(crate) fn replace_cached_preview_revision(&mut self, revision: Option<u64>) {
        self.cached_preview_revision = revision;
    }

    pub(crate) fn clear_cached_preview_revision(&mut self) {
        self.cached_preview_revision = None;
    }

    pub(crate) fn observe_document_revision(&mut self, revision: u64, now: f64) {
        if self.observed_document_revision == Some(revision) {
            return;
        }
        self.observed_document_revision = Some(revision);
        self.last_semantic_edit = now;
    }

    pub(crate) fn revision_is_quiet(&self, revision: u64, now: f64, quiet_period: f64) -> bool {
        self.observed_document_revision == Some(revision)
            && now - self.last_semantic_edit >= quiet_period
    }

    pub(crate) fn should_prepare_revision(
        &self,
        revision: u64,
        now: f64,
        quiet_period: f64,
    ) -> bool {
        self.revision_preparation.is_none()
            && self.last_submitted_revision != Some(revision)
            && self.revision_is_quiet(revision, now, quiet_period)
    }

    pub(crate) fn start_revision_preparation(
        &mut self,
        revision: u64,
        graph: GraphState,
    ) -> Result<(), platform_runtime::WorkExecutorError> {
        self.last_submitted_revision = Some(revision);
        let task = match self
            .graph_service
            .start_revision_preparation(revision, graph)
        {
            Ok(task) => task,
            Err(error) => {
                if error == platform_runtime::WorkExecutorError::QueueFull {
                    self.last_submitted_revision = None;
                }
                return Err(error);
            }
        };
        self.revision_preparation = Some(task);
        Ok(())
    }

    pub(crate) fn poll_revision_preparation(&mut self) -> Option<PreparedGraphRevision> {
        let prepared = self.revision_preparation.as_mut()?.poll()?;
        self.revision_preparation = None;
        Some(prepared)
    }

    pub(crate) fn revision_preparation_pending(&self) -> bool {
        self.revision_preparation.is_some()
    }

    pub(crate) fn progress_update_due(&self, now: f64, interval: f64) -> bool {
        now - self.last_progress_update >= interval
    }

    pub(crate) fn mark_progress_updated(&mut self, now: f64) {
        self.last_progress_update = now;
    }

    pub(crate) fn sampling_overlay_candidates(&self) -> &[SamplingOverlayCandidate] {
        &self.sampling_overlay_candidates
    }

    pub(crate) fn replace_sampling_overlay_candidates(
        &mut self,
        candidates: Vec<SamplingOverlayCandidate>,
    ) {
        self.sampling_overlay_candidates = candidates;
    }

    pub(crate) fn clear_sampling_overlay_candidates(&mut self) {
        self.sampling_overlay_candidates.clear();
    }

    pub(crate) fn cache_clear_task(&self) -> Option<&DerivedCacheClearTask> {
        self.derived_cache_clear_task.as_ref()
    }

    pub(crate) fn cache_clear_task_mut(&mut self) -> Option<&mut DerivedCacheClearTask> {
        self.derived_cache_clear_task.as_mut()
    }

    pub(crate) fn install_cache_clear_task(&mut self, task: DerivedCacheClearTask) {
        self.derived_cache_clear_task = Some(task);
    }

    pub(crate) fn clear_cache_clear_task(&mut self) {
        self.derived_cache_clear_task = None;
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::graph_service::standard_graph_service;

    #[test]
    fn semantic_edits_reset_the_true_debounce_window() {
        let mut lifecycle = GraphRunLifecycle::new(standard_graph_service());
        lifecycle.observe_document_revision(1, 1.0);
        assert!(!lifecycle.should_prepare_revision(1, 1.249, 0.25));
        assert!(lifecycle.should_prepare_revision(1, 1.25, 0.25));

        lifecycle.observe_document_revision(2, 1.2);
        assert!(!lifecycle.should_prepare_revision(2, 1.449, 0.25));
        assert!(lifecycle.should_prepare_revision(2, 1.45, 0.25));
    }

    #[test]
    fn completed_preparation_keeps_its_original_revision_for_stale_rejection() {
        let mut lifecycle = GraphRunLifecycle::new(standard_graph_service());
        lifecycle.observe_document_revision(1, 0.0);
        lifecycle
            .start_revision_preparation(1, GraphState::default())
            .unwrap();
        lifecycle.observe_document_revision(2, 0.3);

        let prepared = lifecycle
            .poll_revision_preparation()
            .expect("inline preparation completes immediately");
        assert_eq!(prepared.revision, 1);
        assert_ne!(prepared.revision, 2);
    }
}
