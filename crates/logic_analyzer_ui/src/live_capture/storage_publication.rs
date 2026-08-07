//! Publication, retention, replay, and export owner for capture artifacts.
//!
//! This module owns completed-session pins, waveform worker retirement, application metadata,
//! replay factories, repository cleanup, and export state behind `CapturePublication`. It consumes
//! generic artifact, capture-session, graph-source, and export contracts. It does not schedule
//! acquisition, order capture commands, project UI status, or contain host adapters.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use logic_analyzer_graph_capabilities::node::CaptureGraphSourceFactory;
use platform_artifacts::{ArtifactKey, ArtifactNamespace, ArtifactRepository, SourceIdentity};
use signal_capture::{CaptureIndex, CaptureMetadata, CaptureSampledWindow};
use signal_capture_session::{
    CaptureCompletion, CaptureRecordingGate, CaptureSessionId, CaptureSessionOutcome,
    CaptureSessionPin, CaptureSessionPlan, CaptureSessionRepository, CaptureSessionSummary,
    CaptureStore, FinalizedCapture, GrowingCaptureIndex, GrowingCaptureIndexWorker,
};

use super::implementation::{
    CaptureAnalysisAttachment, CaptureReplayAttachment, CaptureWaveformUpdate,
    ConfigurationEpochResolution,
};
use crate::capture_export_service::{
    CaptureExportCompletion, CaptureExportFormat as CaptureRawExportFormat, CaptureExportService,
    CaptureExportServiceError, CaptureExportStatus,
};

const APPLICATION_METADATA_NAMESPACE: &str = "capture-application-v1";
const APPLICATION_METADATA_VERSION: u16 = 2;

/// Application-owned graph and source metadata persisted beside one capture session.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaptureApplicationMetadata {
    pub(crate) format_version: u16,
    pub(crate) source_node: u32,
    pub(crate) source_title: String,
    pub(crate) sample_rate_hz: f64,
    pub(crate) channel_names: Vec<String>,
    pub(crate) graph: node_graph::GraphState,
    #[serde(default)]
    pub(crate) configuration_epochs: Vec<PersistedConfigurationEpoch>,
}

impl CaptureApplicationMetadata {
    pub(crate) fn new(
        source_node: node_graph::NodeId,
        source_title: String,
        sample_rate_hz: f64,
        channel_names: Vec<String>,
        graph: node_graph::GraphState,
    ) -> Self {
        Self {
            format_version: APPLICATION_METADATA_VERSION,
            source_node: source_node.0,
            source_title,
            sample_rate_hz,
            channel_names,
            graph,
            configuration_epochs: Vec::new(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedConfigurationEpoch {
    pub(crate) epoch_id: u64,
    pub(crate) source_sample: u64,
    pub(crate) analysis_sample: u64,
    pub(crate) timestamp_ns: u64,
    pub(crate) graph: node_graph::GraphState,
    pub(crate) outcome: PersistedConfigurationEpochOutcome,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PersistedConfigurationEpochOutcome {
    Pending,
    Applied,
    Deferred,
    Failed,
}

/// Finalized artifacts and replay dependencies handed from acquisition to publication.
pub(crate) struct PublishedCapture {
    pub(crate) _session_pin: CaptureSessionPin,
    pub(crate) capture: FinalizedCapture,
    pub(crate) waveform: GrowingCaptureIndex,
    pub(crate) source_node: node_graph::NodeId,
    pub(crate) graph_source_factory: Arc<dyn CaptureGraphSourceFactory>,
    pub(crate) recording_origin: Option<u64>,
    pub(crate) session_plan: Option<CaptureSessionPlan>,
    pub(crate) outcome: CaptureSessionOutcome,
    pub(crate) completion: Option<CaptureCompletion>,
    pub(crate) waveform_worker: Option<GrowingCaptureIndexWorker>,
}

struct PinnedCaptureIndex {
    inner: GrowingCaptureIndex,
    _session_pin: CaptureSessionPin,
}

impl CaptureIndex for PinnedCaptureIndex {
    fn display_name(&self) -> String {
        self.inner.display_name()
    }

    fn index_identity(&self) -> SourceIdentity {
        self.inner.index_identity()
    }

    fn header(&self) -> &CaptureMetadata {
        self.inner.header()
    }

    fn current_metadata(&self) -> CaptureMetadata {
        self.inner.current_metadata()
    }

    fn generation(&self) -> u64 {
        self.inner.generation()
    }

    fn is_complete(&self) -> bool {
        self.inner.is_complete()
    }

    fn capture_duration_us(&self) -> f64 {
        self.inner.capture_duration_us()
    }

    fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> signal_capture::Result<CaptureSampledWindow> {
        self.inner
            .sampled_window(channels, start_sample, end_sample, target_points)
    }
}

/// Owns capture artifacts after acquisition publishes them to the application.
///
/// The owner keeps session pins, waveform workers, replay factories, retention cleanup, and export
/// state coherent behind methods used by `CaptureCoordinator`. It consumes the capture repository
/// and export-service contracts and excludes acquisition commands and UI status projection.
pub(crate) struct CapturePublication {
    repository: CaptureSessionRepository,
    recent_sessions: Vec<CaptureSessionSummary>,
    completed: Option<PublishedCapture>,
    retired: Vec<PublishedCapture>,
    waveform_update: Option<CaptureWaveformUpdate>,
    analysis_attachment: Option<CaptureAnalysisAttachment>,
    export_service: Box<dyn CaptureExportService>,
}

impl CapturePublication {
    pub(crate) fn new(
        repository: CaptureSessionRepository,
        export_service: Box<dyn CaptureExportService>,
    ) -> Self {
        let (recent_sessions, _) = repository.scan_with_cleanup_plan().unwrap_or_default();
        Self {
            repository,
            recent_sessions,
            completed: None,
            retired: Vec::new(),
            waveform_update: None,
            analysis_attachment: None,
            export_service,
        }
    }

    pub(crate) fn repository(&self) -> &CaptureSessionRepository {
        &self.repository
    }

    pub(crate) fn current_session_id(&self) -> Option<CaptureSessionId> {
        self.completed
            .as_ref()
            .map(|completed| completed.capture.manifest().descriptor.session_id())
    }

    pub(crate) fn export_status(&self) -> Option<&CaptureExportStatus> {
        self.export_service.status()
    }

    pub(crate) fn take_export_notice(
        &mut self,
    ) -> Option<Result<CaptureExportCompletion, CaptureExportServiceError>> {
        self.export_service.take_completion()
    }

    pub(crate) fn start_export_current(
        &mut self,
        format: CaptureRawExportFormat,
        destination: PathBuf,
        acquisition_active: bool,
    ) -> Result<(), String> {
        if acquisition_active {
            return Err("finish the live capture before exporting it".into());
        }
        let session_id = self
            .current_session_id()
            .ok_or_else(|| "there is no displayed capture to export".to_owned())?;
        self.export_service
            .start(session_id, format, destination)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn request_cancel_export(&mut self) {
        self.export_service.request_cancel();
    }

    pub(crate) fn poll_export(&mut self) {
        self.export_service.poll();
    }

    pub(crate) fn discard_all_capture_data(
        &mut self,
        acquisition_active: bool,
    ) -> Result<(), String> {
        if acquisition_active {
            return Err("cannot replace capture data while acquisition is active".into());
        }
        if self.export_service.is_active() {
            return Err("cannot replace capture data while it is being saved".into());
        }

        self.analysis_attachment = None;
        self.waveform_update = None;
        self.export_service.reset();

        let mut completed = self.completed.take().into_iter().collect::<Vec<_>>();
        completed.append(&mut self.retired);
        for capture in &mut completed {
            if let Some(worker) = capture.waveform_worker.take() {
                worker.join().map_err(|error| {
                    format!("could not finish the previous capture index: {error}")
                })?;
            }
        }
        drop(completed);

        self.refresh_recent_sessions();
        let session_ids = self
            .recent_sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>();
        for session_id in session_ids {
            let _ = self
                .repository
                .artifact_repository()
                .remove(&application_metadata_key(session_id)?);
            self.repository
                .discard(session_id)
                .map_err(|error| format!("could not remove previous capture data: {error}"))?;
        }
        self.refresh_recent_sessions();
        Ok(())
    }

    pub(crate) fn clear_completed(&mut self) {
        self.retire_completed();
        self.waveform_update = Some(None);
    }

    pub(crate) fn publish_waveform(
        &mut self,
        session_id: CaptureSessionId,
        waveform: GrowingCaptureIndex,
    ) -> Result<(), String> {
        let session_pin = self
            .repository
            .pin(session_id)
            .map_err(|error| format!("could not pin capture waveform: {error}"))?;
        self.waveform_update = Some(Some(Box::new(PinnedCaptureIndex {
            inner: waveform,
            _session_pin: session_pin,
        })));
        Ok(())
    }

    pub(crate) fn publish_analysis(&mut self, analysis: CaptureAnalysisAttachment) {
        self.analysis_attachment = Some(analysis);
    }

    pub(crate) fn publish_completed(&mut self, completed: PublishedCapture) {
        self.retire_completed();
        self.completed = Some(completed);
        self.refresh_recent_sessions();
    }

    pub(crate) fn retain_previous_after_failure(&mut self) -> Result<(), String> {
        let previous = self.completed.as_ref().map(|completed| {
            (
                completed.capture.manifest().descriptor.session_id(),
                completed.waveform.clone(),
            )
        });
        if let Some((session_id, waveform)) = previous {
            self.publish_waveform(session_id, waveform)?;
        }
        self.refresh_recent_sessions();
        Ok(())
    }

    pub(crate) fn reap_waveform_workers(&mut self) -> Option<String> {
        let mut error = None;
        if let Some(completed) = &mut self.completed
            && completed
                .waveform_worker
                .as_ref()
                .is_some_and(GrowingCaptureIndexWorker::is_finished)
            && let Some(worker) = completed.waveform_worker.take()
            && let Err(worker_error) = worker.join()
        {
            error = Some(format!(
                "could not rebuild capture waveform: {worker_error}"
            ));
        }
        let mut pending = Vec::new();
        for mut completed in self.retired.drain(..) {
            let finished = completed
                .waveform_worker
                .as_ref()
                .is_none_or(GrowingCaptureIndexWorker::is_finished);
            if finished {
                if let Some(worker) = completed.waveform_worker.take() {
                    let _ = worker.join();
                }
            } else {
                pending.push(completed);
            }
        }
        self.retired = pending;
        error
    }

    pub(crate) fn take_waveform_update(&mut self) -> Option<CaptureWaveformUpdate> {
        self.waveform_update.take()
    }

    pub(crate) fn take_analysis_attachment(&mut self) -> Option<CaptureAnalysisAttachment> {
        self.analysis_attachment.take()
    }

    pub(crate) fn replay_source_node(&self) -> Option<node_graph::NodeId> {
        self.completed.as_ref().map(|capture| capture.source_node)
    }

    pub(crate) fn create_replay_attachment(
        &self,
    ) -> Result<Option<CaptureReplayAttachment>, String> {
        let Some(completed) = &self.completed else {
            return Ok(None);
        };
        let cursor = completed
            .capture
            .open_cursor()
            .map_err(|error| format!("could not open finalized capture: {error}"))?;
        let cursor =
            CaptureRecordingGate::finalized(completed.recording_origin).cursor(Box::new(cursor));
        let process = completed
            .graph_source_factory
            .create(Box::new(cursor))
            .map_err(|error| format!("could not build capture replay source: {error}"))?;
        Ok(Some(CaptureReplayAttachment {
            source_node: completed.source_node,
            process,
        }))
    }

    #[cfg(test)]
    pub(crate) fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        self.repository.artifact_repository()
    }

    #[cfg(test)]
    pub(crate) fn completed_manifest(
        &self,
    ) -> Option<signal_capture_session::CaptureStoreManifest> {
        self.completed
            .as_ref()
            .map(|completed| completed.capture.manifest())
    }

    #[cfg(test)]
    pub(crate) fn completed_recording_origin(&self) -> Option<u64> {
        self.completed
            .as_ref()
            .and_then(|completed| completed.recording_origin)
    }

    #[cfg(test)]
    pub(crate) fn completed_trigger_sample(&self) -> Option<u64> {
        self.completed.as_ref().and_then(|completed| {
            CaptureIndex::current_metadata(&completed.waveform).trigger_sample
        })
    }

    #[cfg(test)]
    pub(crate) fn completed_session_plan(&self) -> Option<&CaptureSessionPlan> {
        self.completed
            .as_ref()
            .and_then(|completed| completed.session_plan.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn completed_persisted_session_plan(&self) -> Option<CaptureSessionPlan> {
        self.completed
            .as_ref()
            .and_then(|completed| completed.capture.session_plan().ok().flatten())
    }

    fn retire_completed(&mut self) {
        let Some(completed) = self.completed.take() else {
            return;
        };
        if completed
            .waveform_worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            self.retired.push(completed);
        }
    }

    fn refresh_recent_sessions(&mut self) {
        if let Ok((sessions, _)) = self.repository.scan_with_cleanup_plan() {
            self.recent_sessions = sessions;
        }
    }
}

pub(crate) fn write_application_metadata(
    repository: &dyn ArtifactRepository,
    session_id: CaptureSessionId,
    metadata: &CaptureApplicationMetadata,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|error| format!("could not encode capture application metadata: {error}"))?;
    bytes.push(b'\n');
    let mut writer = repository
        .begin_write(application_metadata_key(session_id)?)
        .map_err(|error| error.to_string())?;
    writer
        .write_at(0, &bytes)
        .map_err(|error| error.to_string())?;
    writer
        .truncate(bytes.len() as u64)
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())?;
    writer.publish().map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn read_application_metadata(
    repository: &dyn ArtifactRepository,
    session_id: CaptureSessionId,
) -> Result<CaptureApplicationMetadata, String> {
    let mut reader = repository
        .open(&application_metadata_key(session_id)?)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "capture application metadata is missing".to_owned())?;
    let length = usize::try_from(reader.len().map_err(|error| error.to_string())?)
        .map_err(|_| "capture application metadata is too large".to_owned())?;
    let mut bytes = vec![0_u8; length];
    let mut copied = 0;
    while copied < bytes.len() {
        let count = reader
            .read_at(copied as u64, &mut bytes[copied..])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("capture application metadata is truncated".into());
        }
        copied += count;
    }
    let mut metadata = serde_json::from_slice::<CaptureApplicationMetadata>(&bytes)
        .map_err(|error| format!("invalid capture application metadata: {error}"))?;
    if metadata.format_version != 1 && metadata.format_version != APPLICATION_METADATA_VERSION {
        return Err(format!(
            "unsupported capture application metadata version {}",
            metadata.format_version
        ));
    }
    let mut repaired = metadata.format_version != APPLICATION_METADATA_VERSION;
    metadata.format_version = APPLICATION_METADATA_VERSION;
    for epoch in &mut metadata.configuration_epochs {
        if epoch.outcome == PersistedConfigurationEpochOutcome::Pending {
            epoch.outcome = PersistedConfigurationEpochOutcome::Failed;
            epoch.message = Some("capture ended before this epoch outcome was recorded".into());
            repaired = true;
        }
    }
    if repaired {
        write_application_metadata(repository, session_id, &metadata)?;
    }
    Ok(metadata)
}

pub(crate) fn application_metadata_key(
    session_id: CaptureSessionId,
) -> Result<ArtifactKey, String> {
    let namespace = ArtifactNamespace::new(APPLICATION_METADATA_NAMESPACE)
        .map_err(|error| error.to_string())?;
    let session = session_id.get().to_le_bytes();
    let mut identity = [0_u8; 32];
    identity[..16].copy_from_slice(&session);
    identity[16..].copy_from_slice(&session);
    Ok(ArtifactKey::new(
        namespace,
        SourceIdentity::from_bytes(identity),
    ))
}

pub(crate) fn prepare_configuration_epoch(
    metadata: &mut Option<CaptureApplicationMetadata>,
    repository: &dyn ArtifactRepository,
    session_id: CaptureSessionId,
    graph: node_graph::GraphState,
    store: &CaptureStore,
    recording_gate: &CaptureRecordingGate,
    sample_rate_hz: f64,
) -> Result<super::acquisition_state::WorkerPreparedConfigurationEpoch, String> {
    let metadata = metadata
        .as_mut()
        .ok_or_else(|| "capture graph metadata is unavailable".to_owned())?;
    let recording_origin = recording_gate
        .recording_origin()
        .ok_or_else(|| "capture has not reached its recording origin".to_owned())?;
    let source_sample = store.snapshot().committed_samples.max(recording_origin);
    let analysis_sample = source_sample.saturating_sub(recording_origin);
    let timestamp_step_ns = (1_000_000_000.0 / sample_rate_hz).round();
    if !timestamp_step_ns.is_finite()
        || timestamp_step_ns <= 0.0
        || timestamp_step_ns > u64::MAX as f64
    {
        return Err(format!(
            "capture sample rate {sample_rate_hz} Hz cannot represent an epoch timestamp"
        ));
    }
    let timestamp_ns = source_sample.saturating_mul(timestamp_step_ns as u64);
    let epoch_id = metadata
        .configuration_epochs
        .last()
        .map_or(Ok(1), |epoch| epoch.epoch_id.checked_add(1).ok_or(()))
        .map_err(|()| "configuration epoch ID overflow".to_owned())?;
    metadata.graph = graph.clone();
    metadata
        .configuration_epochs
        .push(PersistedConfigurationEpoch {
            epoch_id,
            source_sample,
            analysis_sample,
            timestamp_ns,
            graph,
            outcome: PersistedConfigurationEpochOutcome::Pending,
            message: None,
        });
    write_application_metadata(repository, session_id, metadata)?;
    Ok(super::acquisition_state::WorkerPreparedConfigurationEpoch {
        epoch_id,
        source_sample,
        boundary: signal_runtime::ConfigurationBoundary::new(source_sample, timestamp_ns),
    })
}

pub(crate) fn resolve_configuration_epoch(
    metadata: &mut Option<CaptureApplicationMetadata>,
    repository: &dyn ArtifactRepository,
    session_id: CaptureSessionId,
    epoch_id: u64,
    resolution: ConfigurationEpochResolution,
) -> Result<(), String> {
    let metadata = metadata
        .as_mut()
        .ok_or_else(|| "capture graph metadata is unavailable".to_owned())?;
    let epoch = metadata
        .configuration_epochs
        .iter_mut()
        .find(|epoch| epoch.epoch_id == epoch_id)
        .ok_or_else(|| format!("configuration epoch {epoch_id} is missing"))?;
    if epoch.outcome != PersistedConfigurationEpochOutcome::Pending {
        return Err(format!(
            "configuration epoch {epoch_id} is already resolved"
        ));
    }
    let (outcome, message) = match resolution {
        ConfigurationEpochResolution::Applied => {
            (PersistedConfigurationEpochOutcome::Applied, None)
        }
        ConfigurationEpochResolution::Deferred(message) => {
            (PersistedConfigurationEpochOutcome::Deferred, Some(message))
        }
        ConfigurationEpochResolution::Failed(message) => {
            (PersistedConfigurationEpochOutcome::Failed, Some(message))
        }
    };
    epoch.outcome = outcome;
    epoch.message = message;
    write_application_metadata(repository, session_id, metadata)
}
