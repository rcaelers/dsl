use logic_analyzer_graph_plan::SamplingOverlayCandidate;
use logic_analyzer_graph_runtime::DerivedCacheClearTask;
use node_graph::GraphState;

use crate::graph_service::{GraphRun, GraphRunFailure, UiGraphService};

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
    running_graph_semantics: Option<Vec<u8>>,
    cached_preview_graph: Option<Vec<u8>>,
    last_live_sync: f64,
    sampling_overlay_candidates: Vec<SamplingOverlayCandidate>,
    derived_cache_clear_task: Option<DerivedCacheClearTask>,
}

impl GraphRunLifecycle {
    pub(crate) fn new(graph_service: UiGraphService) -> Self {
        Self {
            graph_service,
            run: None,
            run_message: None,
            running_graph_semantics: None,
            cached_preview_graph: None,
            last_live_sync: -1.0,
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
        self.running_graph_semantics = None;
        self.run.take()
    }

    pub(crate) fn install_run(&mut self, run: Box<dyn GraphRun>, semantics: Vec<u8>) {
        self.run = Some(run);
        self.running_graph_semantics = Some(semantics);
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

    pub(crate) fn apply_run(
        &mut self,
        graph: &GraphState,
    ) -> Option<
        Result<
            logic_analyzer_graph_runtime::ApplySummary,
            logic_analyzer_graph_runtime::ApplyError,
        >,
    > {
        let run = self.run.as_deref_mut()?;
        Some(self.graph_service.apply_run(run, graph))
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

    pub(crate) fn running_graph_semantics(&self) -> Option<&[u8]> {
        self.running_graph_semantics.as_deref()
    }

    pub(crate) fn set_running_graph_semantics(&mut self, semantics: Vec<u8>) {
        self.running_graph_semantics = Some(semantics);
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

    pub(crate) fn cached_preview_graph(&self) -> Option<&[u8]> {
        self.cached_preview_graph.as_deref()
    }

    pub(crate) fn set_cached_preview_graph(&mut self, revision: Vec<u8>) {
        self.cached_preview_graph = Some(revision);
    }

    pub(crate) fn replace_cached_preview_graph(&mut self, revision: Option<Vec<u8>>) {
        self.cached_preview_graph = revision;
    }

    pub(crate) fn clear_cached_preview_graph(&mut self) {
        self.cached_preview_graph = None;
    }

    pub(crate) fn last_live_sync(&self) -> f64 {
        self.last_live_sync
    }

    pub(crate) fn mark_live_sync(&mut self, now: f64) {
        self.last_live_sync = now;
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
