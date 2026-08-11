//! Thin composition of live-capture acquisition, publication, and status owners.
//!
//! This module owns only transitions that couple those owners and exposes `CaptureCoordinator` as
//! the sibling-facing facade. It consumes the three owner facades plus injected repository,
//! executor, and export services. It does not implement worker commands, artifact retention,
//! status projection, host adapters, or concrete capture behavior.

use std::path::PathBuf;
use std::sync::Arc;

use logic_analyzer_graph_compiler::DiscoveredLiveCaptureFeature;
use platform_artifacts::ArtifactRepository;
use platform_runtime::WorkExecutor;
use signal_capture_session::{
    CaptureSessionId, CaptureSessionRepository, CaptureSessionRepositoryConfig, CaptureStartMode,
};

#[cfg(test)]
use super::acquisition_state::waveform_ready_for_publication;
use super::acquisition_state::{CaptureAcquisition, WorkerCompletion};
use super::error::CaptureCoordinatorError;
use super::implementation::{
    CaptureAnalysisAttachment, CaptureCoordinatorContract, CaptureReplayAttachment,
    CaptureSessionStatus, CaptureWaveformUpdate, ConfigurationEpochResolution,
    PreparedConfigurationEpoch,
};
use super::status_projection::CaptureStatusProjection;
use super::storage_publication::CapturePublication;
#[cfg(test)]
use super::storage_publication::{PersistedConfigurationEpochOutcome, read_application_metadata};
use crate::capture_export_service::{
    CaptureExportCompletion, CaptureExportFormat as CaptureRawExportFormat, CaptureExportService,
    CaptureExportServiceError, CaptureExportStatus,
};
#[cfg(test)]
use crate::capture_export_service::{
    ScriptedCaptureExportControl, scripted_capture_export_service,
};

/// Composes capture acquisition, artifact publication, and UI status projection.
///
/// The coordinator owns only transitions that couple two or more capture owners and implements the
/// stable `CaptureCoordinatorContract` facade. It depends on those owner facades and injected host
/// services; worker protocol, storage retention, and projected status invariants remain internal to
/// their respective owners.
pub(crate) struct CaptureCoordinator {
    acquisition: CaptureAcquisition,
    publication: CapturePublication,
    projection: CaptureStatusProjection,
}

impl CaptureCoordinator {
    #[cfg(test)]
    fn new() -> Self {
        Self::new_with_scripted_export().0
    }

    #[cfg(test)]
    fn new_with_scripted_export() -> (Self, ScriptedCaptureExportControl) {
        let artifacts = Arc::new(platform_artifacts::MemoryArtifactRepository::new());
        let repository =
            CaptureSessionRepository::new(CaptureSessionRepositoryConfig::new(artifacts))
                .expect("temporary capture repository must be available");
        let (export_service, control) = scripted_capture_export_service();
        (
            Self::with_repository_and_export_service(
                repository,
                export_service,
                coordinator_tests::test_work_executor(),
            ),
            control,
        )
    }

    pub(crate) fn configured(
        max_recent_sessions: usize,
        max_total_bytes: u64,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        export_service: Box<dyn CaptureExportService>,
    ) -> Self {
        let config = CaptureSessionRepositoryConfig::new(artifact_repository)
            .with_limits(max_recent_sessions, max_total_bytes)
            .expect("embedded live-capture limits are valid");
        let repository = CaptureSessionRepository::new(config)
            .expect("the live-capture artifact repository must be available");
        Self::with_repository_and_export_service(repository, export_service, work_executor)
    }

    fn with_repository_and_export_service(
        repository: CaptureSessionRepository,
        export_service: Box<dyn CaptureExportService>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self {
            acquisition: CaptureAcquisition::new(work_executor),
            publication: CapturePublication::new(repository, export_service),
            projection: CaptureStatusProjection::new(),
        }
    }

    pub(crate) fn current_session_id(&self) -> Option<CaptureSessionId> {
        self.publication.current_session_id()
    }

    pub(crate) fn export_status(&self) -> Option<&CaptureExportStatus> {
        self.publication.export_status()
    }

    pub(crate) fn take_export_notice(
        &mut self,
    ) -> Option<Result<CaptureExportCompletion, CaptureExportServiceError>> {
        self.publication.take_export_notice()
    }

    pub(crate) fn start_export_current(
        &mut self,
        format: CaptureRawExportFormat,
        destination: PathBuf,
    ) -> Result<(), CaptureCoordinatorError> {
        self.publication
            .start_export_current(format, destination, self.acquisition.is_active())
    }

    pub(crate) fn request_cancel_export(&mut self) {
        self.publication.request_cancel_export();
    }

    pub(crate) fn start_with_graph(
        &mut self,
        feature: DiscoveredLiveCaptureFeature,
        graph: &node_graph::api::GraphState,
        mode: CaptureStartMode,
    ) -> Result<(), CaptureCoordinatorError> {
        self.start_session(feature, Some(graph), mode)
    }

    fn start_session(
        &mut self,
        feature: DiscoveredLiveCaptureFeature,
        graph: Option<&node_graph::api::GraphState>,
        mode: CaptureStartMode,
    ) -> Result<(), CaptureCoordinatorError> {
        if self.acquisition.is_active() {
            return Err(CaptureCoordinatorError::policy(
                "a live capture is already active",
            ));
        }
        if mode == CaptureStartMode::CaptureNow && !feature.capabilities().commands().capture_now {
            return Err(CaptureCoordinatorError::policy(
                "this capture source does not support Capture Now",
            ));
        }
        self.publication.discard_all_capture_data(false)?;
        self.projection.clear();
        let start =
            self.acquisition
                .start(self.publication.repository().clone(), feature, graph, mode)?;
        self.projection.start(start);
        Ok(())
    }

    pub(crate) fn clear_completed(&mut self) {
        self.publication.clear_completed();
        self.projection.clear();
    }

    fn finish_worker(&mut self, completion: WorkerCompletion) {
        match completion {
            WorkerCompletion::Complete(completed) => {
                self.projection.complete(
                    completed.session_plan.clone(),
                    completed.outcome,
                    completed.completion,
                );
                self.publication.publish_completed(*completed);
            }
            WorkerCompletion::Failed(error) => {
                self.projection.fail(error.to_string());
                if let Err(error) = self.publication.retain_previous_after_failure() {
                    self.projection.report_error(error.to_string());
                }
            }
        }
    }

    #[cfg(test)]
    fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        self.publication.artifact_repository()
    }

    #[cfg(test)]
    fn capture_session_exists(&self, session_id: CaptureSessionId) -> bool {
        self.publication.repository().scan().is_ok_and(|sessions| {
            sessions
                .iter()
                .any(|session| session.session_id == session_id)
        })
    }

    #[cfg(test)]
    fn completed_manifest(&self) -> Option<signal_capture_session::CaptureStoreManifest> {
        self.publication.completed_manifest()
    }

    #[cfg(test)]
    fn completed_recording_origin(&self) -> Option<u64> {
        self.publication.completed_recording_origin()
    }

    #[cfg(test)]
    fn completed_trigger_sample(&self) -> Option<u64> {
        self.publication.completed_trigger_sample()
    }

    #[cfg(test)]
    fn completed_session_plan(&self) -> Option<&signal_capture_session::CaptureSessionPlan> {
        self.publication.completed_session_plan()
    }

    #[cfg(test)]
    fn completed_persisted_session_plan(
        &self,
    ) -> Option<signal_capture_session::CaptureSessionPlan> {
        self.publication.completed_persisted_session_plan()
    }

    #[cfg(test)]
    fn state_history(&self) -> &[signal_capture_session::CaptureSessionState] {
        self.projection.state_history()
    }
}

impl CaptureCoordinatorContract for CaptureCoordinator {
    fn backend_unavailable_reason(&self) -> Option<&'static str> {
        self.acquisition.backend_unavailable_reason()
    }

    fn request_stop(&mut self) {
        if self.projection.supports_orderly_stop() && self.acquisition.request_stop() {
            self.projection.mark_stopping();
        }
    }

    fn request_abort(&mut self) -> Result<(), CaptureCoordinatorError> {
        self.projection
            .ensure_abort_supported()
            .map_err(CaptureCoordinatorError::policy)?;
        self.acquisition.request_abort()
    }

    fn request_force_trigger(&mut self) -> Result<(), CaptureCoordinatorError> {
        self.projection
            .ensure_force_trigger_supported()
            .map_err(CaptureCoordinatorError::policy)?;
        self.acquisition.request_force_trigger()
    }

    fn set_graph_processed_samples(&mut self, processed_samples: Option<u64>) {
        self.projection
            .set_graph_processed_samples(processed_samples);
    }

    fn poll(&mut self) {
        self.publication.poll_export();
        if let Some(error) = self.publication.reap_waveform_workers() {
            self.projection.report_error(error.to_string());
        }

        let poll = self.acquisition.poll();
        if let Some(analysis) = poll.analysis {
            self.publication.publish_analysis(analysis);
        }
        if let Some(waveform) = poll.waveform {
            let result = self
                .projection
                .session_id()
                .ok_or_else(|| {
                    CaptureCoordinatorError::protocol(
                        "capture status is unavailable for its waveform",
                    )
                })
                .and_then(|session_id| self.publication.publish_waveform(session_id, waveform));
            if let Err(error) = result {
                self.projection.report_error(error.to_string());
            }
        }
        let stop_requested = self.acquisition.stop_requested();
        for event in poll.events {
            self.projection.apply_event(event, stop_requested);
        }
        if let Some(completion) = poll.completion {
            self.finish_worker(completion);
        }
    }

    fn status(&self) -> Option<&CaptureSessionStatus> {
        self.projection.status()
    }

    fn take_waveform_update(&mut self) -> Option<CaptureWaveformUpdate> {
        self.publication.take_waveform_update()
    }

    fn take_analysis_attachment(&mut self) -> Option<CaptureAnalysisAttachment> {
        self.publication.take_analysis_attachment()
    }

    fn request_configuration_epoch(
        &mut self,
        graph: node_graph::api::GraphState,
    ) -> Result<(), CaptureCoordinatorError> {
        if !self.projection.is_recording() {
            return Err(CaptureCoordinatorError::policy(
                "configuration changes are accepted only while recording",
            ));
        }
        self.acquisition.request_configuration_epoch(graph)
    }

    fn take_configuration_epoch_preparation(
        &mut self,
    ) -> Option<Result<PreparedConfigurationEpoch, CaptureCoordinatorError>> {
        self.acquisition.take_configuration_epoch_preparation()
    }

    fn resolve_configuration_epoch(
        &mut self,
        epoch_id: u64,
        resolution: ConfigurationEpochResolution,
    ) -> Result<(), CaptureCoordinatorError> {
        self.acquisition
            .resolve_configuration_epoch(epoch_id, resolution)
    }

    fn take_configuration_epoch_notice(&mut self) -> Option<Result<(), CaptureCoordinatorError>> {
        self.acquisition.take_configuration_epoch_notice()
    }

    fn replay_source_node(&self) -> Option<node_graph::api::NodeId> {
        self.publication.replay_source_node()
    }

    fn create_replay_attachment(
        &self,
    ) -> Result<Option<CaptureReplayAttachment>, CaptureCoordinatorError> {
        self.publication.create_replay_attachment()
    }

    fn is_active(&self) -> bool {
        self.acquisition.is_active()
    }

    fn graph_editing_enabled(&self) -> bool {
        self.projection
            .graph_editing_enabled(self.acquisition.is_active())
    }
}

#[cfg(test)]
mod coordinator_tests;
