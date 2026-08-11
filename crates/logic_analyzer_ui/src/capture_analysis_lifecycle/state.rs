use logic_analyzer_graph_compiler::DiscoveredTriggerConfiguration;
use node_graph::GraphState;

use crate::graph_service::GraphRun;
use crate::live_capture::{CaptureAvailability, CaptureCoordinator};
use crate::memory_panel::CaptureStorageSnapshot;

/// Owns the mutable state that spans capture acquisition and post-capture analysis.
///
/// `analysis_error` is populated only when no analysis run is installed. A capture graph snapshot
/// and its observed semantic revision are established together at acquisition start and cleared
/// together when capture work becomes inactive. Configuration-epoch requests are serialized by the
/// `epoch_request_in_flight` flag.
pub(crate) struct CaptureAnalysisLifecycle {
    coordinator: CaptureCoordinator,
    availability: CaptureAvailability,
    trigger_configuration: Option<DiscoveredTriggerConfiguration>,
    trigger_configuration_error: Option<String>,
    capture_graph: Option<GraphState>,
    analysis: Option<Box<dyn GraphRun>>,
    analysis_error: Option<String>,
    epoch_observed_graph: Option<Vec<u8>>,
    epoch_request_in_flight: bool,
    last_epoch_sync: f64,
    presentation_identity: Option<String>,
    storage: Option<CaptureStorageSnapshot>,
}

impl CaptureAnalysisLifecycle {
    pub(crate) fn new(coordinator: CaptureCoordinator, availability: CaptureAvailability) -> Self {
        Self {
            coordinator,
            availability,
            trigger_configuration: None,
            trigger_configuration_error: Some(
                "Checking the graph for a trigger-configurable source".into(),
            ),
            capture_graph: None,
            analysis: None,
            analysis_error: None,
            epoch_observed_graph: None,
            epoch_request_in_flight: false,
            last_epoch_sync: -1.0,
            presentation_identity: None,
            storage: None,
        }
    }

    pub(crate) fn coordinator(&self) -> &CaptureCoordinator {
        &self.coordinator
    }

    pub(crate) fn coordinator_mut(&mut self) -> &mut CaptureCoordinator {
        &mut self.coordinator
    }

    pub(crate) fn availability(&self) -> &CaptureAvailability {
        &self.availability
    }

    pub(crate) fn set_availability(&mut self, availability: CaptureAvailability) {
        self.availability = availability;
    }

    pub(crate) fn trigger_configuration(&self) -> Option<&DiscoveredTriggerConfiguration> {
        self.trigger_configuration.as_ref()
    }

    pub(crate) fn trigger_configuration_error(&self) -> Option<&str> {
        self.trigger_configuration_error.as_deref()
    }

    pub(crate) fn set_trigger_configuration(
        &mut self,
        configuration: Option<DiscoveredTriggerConfiguration>,
    ) {
        self.trigger_configuration_error = configuration
            .is_none()
            .then(|| "The graph has no trigger-configurable source".into());
        self.trigger_configuration = configuration;
    }

    pub(crate) fn set_trigger_configuration_error(&mut self, error: impl Into<String>) {
        self.trigger_configuration = None;
        self.trigger_configuration_error = Some(error.into());
    }

    pub(crate) fn begin_capture(&mut self, graph: GraphState, observed_graph: Option<Vec<u8>>) {
        self.capture_graph = Some(graph);
        self.epoch_observed_graph = observed_graph;
        self.epoch_request_in_flight = false;
        self.clear_analysis();
    }

    pub(crate) fn clear_capture_graph(&mut self) {
        self.capture_graph = None;
        self.epoch_observed_graph = None;
        self.epoch_request_in_flight = false;
    }

    pub(crate) fn take_capture_graph(&mut self) -> Option<GraphState> {
        self.capture_graph.take()
    }

    pub(crate) fn analysis(&self) -> Option<&dyn GraphRun> {
        self.analysis.as_deref()
    }

    pub(crate) fn analysis_mut(&mut self) -> Option<&mut (dyn GraphRun + 'static)> {
        self.analysis.as_deref_mut()
    }

    pub(crate) fn install_analysis(&mut self, analysis: Box<dyn GraphRun>) {
        self.analysis = Some(analysis);
        self.analysis_error = None;
    }

    pub(crate) fn clear_analysis(&mut self) {
        self.analysis = None;
        self.analysis_error = None;
    }

    pub(crate) fn fail_analysis(&mut self, error: impl Into<String>) {
        self.analysis = None;
        self.analysis_error = Some(error.into());
    }

    pub(crate) fn analysis_error(&self) -> Option<&str> {
        self.analysis_error.as_deref()
    }

    pub(crate) fn is_analysis_active(&self) -> bool {
        self.analysis.as_ref().is_some_and(|run| !run.is_finished())
    }

    pub(crate) fn epoch_observed_graph(&self) -> Option<&[u8]> {
        self.epoch_observed_graph.as_deref()
    }

    pub(crate) fn observe_epoch_graph(&mut self, revision: Vec<u8>) {
        self.epoch_observed_graph = Some(revision);
    }

    pub(crate) fn epoch_request_in_flight(&self) -> bool {
        self.epoch_request_in_flight
    }

    pub(crate) fn mark_epoch_request_started(&mut self) {
        self.epoch_request_in_flight = true;
    }

    pub(crate) fn mark_epoch_request_finished(&mut self) {
        self.epoch_request_in_flight = false;
    }

    pub(crate) fn last_epoch_sync(&self) -> f64 {
        self.last_epoch_sync
    }

    pub(crate) fn mark_epoch_sync(&mut self, now: f64) {
        self.last_epoch_sync = now;
    }

    pub(crate) fn presentation_identity(&self) -> Option<&str> {
        self.presentation_identity.as_deref()
    }

    pub(crate) fn set_presentation_identity(&mut self, identity: Option<String>) {
        self.presentation_identity = identity;
    }

    pub(crate) fn storage(&self) -> Option<&CaptureStorageSnapshot> {
        self.storage.as_ref()
    }

    pub(crate) fn set_storage(&mut self, storage: CaptureStorageSnapshot) {
        self.storage = Some(storage);
    }

    pub(crate) fn clear_storage(&mut self) {
        self.storage = None;
    }
}
