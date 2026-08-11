//! Active-capture worker and command-protocol owner.
//!
//! This module owns the single-active-worker invariant and ordered configuration-epoch
//! acknowledgements behind `CaptureAcquisition`. It consumes provider-neutral capture, session
//! repository, and work-executor contracts. It does not retain published sessions, run exports,
//! project UI status, or know concrete devices and protocols.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use web_time::{Instant, SystemTime, UNIX_EPOCH};

use logic_analyzer_graph_compiler::DiscoveredLiveCaptureFeature;
use platform_runtime::WorkExecutor;
use signal_capture::CaptureIndex;
use signal_capture_session::{
    AcquisitionContext, CaptureCompletion, CaptureDataDelivery, CaptureEvent,
    CaptureEventPublishError, CaptureEventPublisher, CaptureEventQueueReader, CaptureHealth,
    CaptureQueueReceiveError, CaptureRecordingGate, CaptureSessionId, CaptureSessionOutcome,
    CaptureSessionRepository, CaptureStartMode, CaptureStore, CaptureStoreConfig,
    CaptureStoreDescriptor, CaptureTimelineMetadata, GrowingCaptureIndex, RecordingStart,
    TriggerTimeoutAction, bounded_capture_event_queue,
};

use super::error::{
    CaptureAttachmentKind, CaptureCoordinatorError, CaptureStoreOperation, CaptureWaveformOperation,
};
use super::implementation::{
    CaptureAnalysisAttachment, ConfigurationEpochResolution, PreparedConfigurationEpoch,
};
use super::status_projection::CaptureStartProjection;
use super::storage_publication::{
    CaptureApplicationMetadata, PersistedConfigurationEpochOutcome, PublishedCapture,
    application_metadata_key, prepare_configuration_epoch, resolve_configuration_epoch,
    write_application_metadata,
};

const EVENT_QUEUE_CAPACITY: usize = 1_024;
const SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(5);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct WorkerPreparedConfigurationEpoch {
    pub(crate) epoch_id: u64,
    pub(crate) source_sample: u64,
    pub(crate) boundary: signal_runtime::ConfigurationBoundary,
}

enum CaptureCommand {
    Stop,
    Abort,
    ForceTrigger,
    PrepareConfigurationEpoch {
        graph: Box<node_graph::api::GraphState>,
        response: Sender<Result<WorkerPreparedConfigurationEpoch, CaptureCoordinatorError>>,
    },
    ResolveConfigurationEpoch {
        epoch_id: u64,
        resolution: ConfigurationEpochResolution,
        response: Sender<Result<(), CaptureCoordinatorError>>,
    },
}

pub(crate) enum WorkerCompletion {
    Complete(Box<PublishedCapture>),
    Failed(CaptureCoordinatorError),
}

struct ActiveCapture {
    commands: Sender<CaptureCommand>,
    completion: Receiver<WorkerCompletion>,
    waveforms: Receiver<GrowingCaptureIndex>,
    analyses: Receiver<CaptureAnalysisAttachment>,
    events: CaptureEventQueueReader,
    worker: Option<Box<dyn platform_runtime::WorkTask>>,
    stop_requested: bool,
    abort_requested: bool,
}

struct PendingConfigurationEpoch {
    graph: node_graph::api::GraphState,
    response: Receiver<Result<WorkerPreparedConfigurationEpoch, CaptureCoordinatorError>>,
}

struct CaptureWorkerPorts {
    events: Box<dyn CaptureEventPublisher>,
    commands: Receiver<CaptureCommand>,
    waveform_ready: Sender<GrowingCaptureIndex>,
    analysis_ready: Sender<CaptureAnalysisAttachment>,
}

struct CaptureWorkerSession {
    repository: CaptureSessionRepository,
    application_metadata: Option<CaptureApplicationMetadata>,
}

/// Values emitted by the active acquisition during one non-blocking poll.
pub(crate) struct AcquisitionPoll {
    pub(crate) analysis: Option<CaptureAnalysisAttachment>,
    pub(crate) waveform: Option<GrowingCaptureIndex>,
    pub(crate) events: Vec<CaptureEvent>,
    pub(crate) completion: Option<WorkerCompletion>,
}

/// Owns the single active capture worker and its ordered command/acknowledgement protocol.
///
/// The owner admits at most one active worker, serializes configuration-epoch commands, and emits
/// non-blocking acquisition observations through `poll`. It consumes generic capture, repository,
/// and executor contracts; it excludes published-session retention, export, and UI status state.
pub(crate) struct CaptureAcquisition {
    active: Option<ActiveCapture>,
    pending_configuration_epoch: Option<PendingConfigurationEpoch>,
    configuration_epoch_preparation:
        Option<Result<PreparedConfigurationEpoch, CaptureCoordinatorError>>,
    configuration_epoch_resolutions: Vec<Receiver<Result<(), CaptureCoordinatorError>>>,
    configuration_epoch_notice: Option<Result<(), CaptureCoordinatorError>>,
    work_executor: Arc<dyn WorkExecutor>,
}

impl CaptureAcquisition {
    pub(crate) fn new(work_executor: Arc<dyn WorkExecutor>) -> Self {
        Self {
            active: None,
            pending_configuration_epoch: None,
            configuration_epoch_preparation: None,
            configuration_epoch_resolutions: Vec::new(),
            configuration_epoch_notice: None,
            work_executor,
        }
    }

    pub(crate) fn backend_unavailable_reason(&self) -> Option<&'static str> {
        (!self.work_executor.supports_long_running_tasks())
            .then_some("The selected host cannot schedule live capture supervision")
    }

    pub(crate) fn start(
        &mut self,
        repository: CaptureSessionRepository,
        feature: DiscoveredLiveCaptureFeature,
        graph: Option<&node_graph::api::GraphState>,
        mode: CaptureStartMode,
    ) -> Result<CaptureStartProjection, CaptureCoordinatorError> {
        if self.is_active() {
            return Err(CaptureCoordinatorError::policy(
                "a live capture is already active",
            ));
        }
        let commands = feature.capabilities().commands();
        if mode == CaptureStartMode::CaptureNow && !commands.capture_now {
            return Err(CaptureCoordinatorError::policy(
                "this capture source does not support Capture Now",
            ));
        }
        if !self.work_executor.supports_long_running_tasks() {
            return Err(CaptureCoordinatorError::policy(
                "the selected host cannot schedule live capture supervision",
            ));
        }

        let session_id = fresh_session_id();
        let source_node = feature.source_node();
        let source_title = feature.source_title().to_owned();
        let session_plan = feature.session_plan().cloned().map(|plan| {
            if mode == CaptureStartMode::CaptureNow {
                plan.capture_now()
            } else {
                plan
            }
        });
        let recording_origin = session_plan
            .as_ref()
            .map(|plan| plan.policy.effective.start == RecordingStart::Immediate)
            .unwrap_or_else(|| !feature.has_trigger_program())
            .then_some(0);
        let application_metadata = graph.map(|graph| {
            CaptureApplicationMetadata::new(
                source_node,
                source_title.clone(),
                feature.sample_rate_hz(),
                feature.channel_names().to_vec(),
                graph.clone(),
            )
        });
        let (event_publisher, events) = bounded_capture_event_queue(EVENT_QUEUE_CAPACITY)
            .expect("capture event queue capacity is non-zero");
        let (command_sender, command_receiver) = crossbeam_channel::unbounded();
        let (completion_sender, completion_receiver) = crossbeam_channel::bounded(1);
        let (waveform_sender, waveform_receiver) = crossbeam_channel::bounded(1);
        let (analysis_sender, analysis_receiver) = crossbeam_channel::bounded(1);
        let supervisor_executor = Arc::clone(&self.work_executor);
        let capture_work_executor = Arc::clone(&self.work_executor);
        let worker = supervisor_executor
            .submit_long_running(Box::new(move || {
                let completion = match run_capture_worker(
                    session_id,
                    feature,
                    mode,
                    CaptureWorkerSession {
                        repository,
                        application_metadata,
                    },
                    CaptureWorkerPorts {
                        events: Box::new(event_publisher),
                        commands: command_receiver,
                        waveform_ready: waveform_sender,
                        analysis_ready: analysis_sender,
                    },
                    capture_work_executor,
                ) {
                    Ok(capture) => WorkerCompletion::Complete(Box::new(capture)),
                    Err(error) => WorkerCompletion::Failed(error),
                };
                let _ = completion_sender.send(completion);
            }))
            .map_err(CaptureCoordinatorError::Executor)?;

        self.active = Some(ActiveCapture {
            commands: command_sender,
            completion: completion_receiver,
            waveforms: waveform_receiver,
            analyses: analysis_receiver,
            events,
            worker: Some(worker),
            stop_requested: false,
            abort_requested: false,
        });
        Ok(CaptureStartProjection {
            session_id,
            source_node,
            source_title,
            commands,
            session_plan,
            recording_origin,
        })
    }

    pub(crate) fn request_stop(&mut self) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        if active.stop_requested {
            return false;
        }
        active.stop_requested = true;
        let _ = active.commands.try_send(CaptureCommand::Stop);
        true
    }

    pub(crate) fn request_abort(&mut self) -> Result<(), CaptureCoordinatorError> {
        let active = self.active.as_mut().ok_or_else(|| {
            CaptureCoordinatorError::policy("there is no active capture to abort")
        })?;
        if !active.abort_requested {
            active.abort_requested = true;
            active
                .commands
                .try_send(CaptureCommand::Abort)
                .map_err(|_| {
                    CaptureCoordinatorError::protocol("could not request capture abort")
                })?;
        }
        Ok(())
    }

    pub(crate) fn request_force_trigger(&mut self) -> Result<(), CaptureCoordinatorError> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| CaptureCoordinatorError::policy("there is no armed capture"))?;
        active
            .commands
            .try_send(CaptureCommand::ForceTrigger)
            .map_err(|_| CaptureCoordinatorError::protocol("could not request force trigger"))
    }

    pub(crate) fn poll(&mut self) -> AcquisitionPoll {
        self.poll_configuration_epochs();
        let analysis = self
            .active
            .as_ref()
            .and_then(|active| active.analyses.try_recv().ok());
        let waveform = self
            .active
            .as_ref()
            .and_then(|active| active.waveforms.try_recv().ok());
        let mut events = Vec::new();
        loop {
            let event = self.active.as_ref().map(|active| active.events.try_recv());
            match event {
                Some(Ok(event)) => {
                    let triggered = matches!(event, CaptureEvent::Triggered { .. });
                    events.push(event);
                    if triggered {
                        return AcquisitionPoll {
                            analysis,
                            waveform,
                            events,
                            completion: None,
                        };
                    }
                }
                Some(Err(CaptureQueueReceiveError::Empty | CaptureQueueReceiveError::Closed))
                | None => break,
                Some(Err(CaptureQueueReceiveError::Timeout)) => unreachable!(),
            }
        }
        let completion =
            self.active
                .as_ref()
                .and_then(|active| match active.completion.try_recv() {
                    Ok(completion) => Some(completion),
                    Err(TryRecvError::Disconnected) => {
                        Some(WorkerCompletion::Failed(CaptureCoordinatorError::protocol(
                            "capture supervisor stopped without a result",
                        )))
                    }
                    Err(TryRecvError::Empty) => None,
                });
        if completion.is_some() {
            self.finish_active_worker();
        }
        AcquisitionPoll {
            analysis,
            waveform,
            events,
            completion,
        }
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.stop_requested)
    }

    pub(crate) fn request_configuration_epoch(
        &mut self,
        graph: node_graph::api::GraphState,
    ) -> Result<(), CaptureCoordinatorError> {
        if self.pending_configuration_epoch.is_some()
            || self.configuration_epoch_preparation.is_some()
        {
            return Err(CaptureCoordinatorError::policy(
                "a configuration epoch is already being prepared",
            ));
        }
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| CaptureCoordinatorError::policy("there is no active capture"))?;
        let (response_sender, response) = crossbeam_channel::bounded(1);
        active
            .commands
            .send(CaptureCommand::PrepareConfigurationEpoch {
                graph: Box::new(graph.clone()),
                response: response_sender,
            })
            .map_err(|_| {
                CaptureCoordinatorError::protocol(
                    "capture supervisor no longer accepts configuration changes",
                )
            })?;
        self.pending_configuration_epoch = Some(PendingConfigurationEpoch { graph, response });
        Ok(())
    }

    pub(crate) fn take_configuration_epoch_preparation(
        &mut self,
    ) -> Option<Result<PreparedConfigurationEpoch, CaptureCoordinatorError>> {
        self.configuration_epoch_preparation.take()
    }

    pub(crate) fn resolve_configuration_epoch(
        &mut self,
        epoch_id: u64,
        resolution: ConfigurationEpochResolution,
    ) -> Result<(), CaptureCoordinatorError> {
        let active = self.active.as_ref().ok_or_else(|| {
            CaptureCoordinatorError::policy(
                "capture ended before the configuration epoch was resolved",
            )
        })?;
        let (response_sender, response) = crossbeam_channel::bounded(1);
        active
            .commands
            .send(CaptureCommand::ResolveConfigurationEpoch {
                epoch_id,
                resolution,
                response: response_sender,
            })
            .map_err(|_| {
                CaptureCoordinatorError::protocol(
                    "capture supervisor no longer accepts epoch outcomes",
                )
            })?;
        self.configuration_epoch_resolutions.push(response);
        Ok(())
    }

    pub(crate) fn take_configuration_epoch_notice(
        &mut self,
    ) -> Option<Result<(), CaptureCoordinatorError>> {
        self.configuration_epoch_notice.take()
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn poll_configuration_epochs(&mut self) {
        let preparation =
            self.pending_configuration_epoch
                .as_ref()
                .and_then(|pending| match pending.response.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Err(CaptureCoordinatorError::protocol(
                            "capture supervisor stopped while preparing a configuration epoch",
                        )))
                    }
                });
        if let Some(preparation) = preparation {
            let pending = self
                .pending_configuration_epoch
                .take()
                .expect("preparation came from a pending epoch");
            self.configuration_epoch_preparation =
                Some(preparation.map(|prepared| PreparedConfigurationEpoch {
                    epoch_id: prepared.epoch_id,
                    source_sample: prepared.source_sample,
                    boundary: prepared.boundary,
                    graph: pending.graph,
                }));
        }

        let mut index = 0;
        while index < self.configuration_epoch_resolutions.len() {
            let result = match self.configuration_epoch_resolutions[index].try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(CaptureCoordinatorError::protocol(
                    "capture supervisor stopped before resolving a configuration epoch",
                ))),
            };
            if let Some(result) = result {
                self.configuration_epoch_resolutions.swap_remove(index);
                self.configuration_epoch_notice = Some(result);
            } else {
                index += 1;
            }
        }
    }

    fn finish_active_worker(&mut self) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        if let Some(worker) = active.worker.take() {
            worker.wait();
        }
    }
}

impl Drop for CaptureAcquisition {
    fn drop(&mut self) {
        if let Some(mut active) = self.active.take() {
            let _ = active.commands.try_send(CaptureCommand::Stop);
            drop(active.commands);
            if let Some(worker) = active.worker.take() {
                worker.wait();
            }
        }
    }
}

#[derive(Default)]
struct CaptureRuntimeSignals {
    trigger_sample: Option<u64>,
    captured_samples: u64,
}

struct RecordingEventPublisher {
    inner: Box<dyn CaptureEventPublisher>,
    recording_gate: CaptureRecordingGate,
    waveform: GrowingCaptureIndex,
    store: CaptureStore,
    runtime: Arc<Mutex<CaptureRuntimeSignals>>,
    last_health_at: Instant,
    last_health_bytes: u64,
}

impl CaptureEventPublisher for RecordingEventPublisher {
    fn publish(&mut self, event: CaptureEvent) -> Result<(), CaptureEventPublishError> {
        let progress = match &event {
            CaptureEvent::Triggered { sample, .. } => {
                self.recording_gate.resolve_trigger(*sample);
                self.waveform.set_trigger_sample(*sample);
                self.runtime
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .trigger_sample = Some(*sample);
                None
            }
            CaptureEvent::Progress { progress, .. } => {
                if let Some(samples) = progress.captured_samples {
                    self.runtime
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .captured_samples = samples;
                }
                Some(*progress)
            }
            _ => None,
        };
        self.inner.publish(event)?;
        let elapsed = self.last_health_at.elapsed();
        if let Some(progress) = progress
            && elapsed >= Duration::from_millis(100)
        {
            let transferred = progress.transferred_bytes.unwrap_or(self.last_health_bytes);
            let bytes = transferred.saturating_sub(self.last_health_bytes);
            let rate =
                u64::try_from((u128::from(bytes) * 1_000_000_000_u128) / elapsed.as_nanos().max(1))
                    .unwrap_or(u64::MAX);
            let snapshot = self.store.snapshot();
            let indexed = self.waveform.current_metadata().total_samples;
            let captured = progress.captured_samples.unwrap_or_else(|| {
                self.runtime
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .captured_samples
            });
            let _ = self.inner.publish(CaptureEvent::Health {
                session_id: self.store.descriptor().session_id(),
                health: CaptureHealth {
                    input_bytes_per_second: Some(rate),
                    write_bytes_per_second: Some(rate),
                    stored_samples: Some(snapshot.committed_samples),
                    summary_lag_samples: Some(captured.saturating_sub(indexed)),
                    ..CaptureHealth::default()
                },
            });
            self.last_health_at = Instant::now();
            self.last_health_bytes = transferred;
        }
        Ok(())
    }
}

fn fresh_session_id() -> CaptureSessionId {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = u128::from(NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed));
    CaptureSessionId::new(time.rotate_left(37) ^ sequence)
}

fn run_capture_worker(
    session_id: CaptureSessionId,
    feature: DiscoveredLiveCaptureFeature,
    mode: CaptureStartMode,
    session: CaptureWorkerSession,
    ports: CaptureWorkerPorts,
    work_executor: Arc<dyn WorkExecutor>,
) -> Result<PublishedCapture, CaptureCoordinatorError> {
    let CaptureWorkerSession {
        repository,
        mut application_metadata,
    } = session;
    let session_pin = repository.reserve(session_id).map_err(|error| {
        CaptureCoordinatorError::store(CaptureStoreOperation::ReserveSession, error)
    })?;
    if let Some(metadata) = &application_metadata
        && let Err(error) = write_application_metadata(
            repository.artifact_repository().as_ref(),
            session_id,
            metadata,
        )
    {
        drop(session_pin);
        let _ = repository
            .artifact_repository()
            .remove(&application_metadata_key(session_id)?);
        let _ = repository.discard(session_id);
        return Err(error);
    }
    let CaptureWorkerPorts {
        events,
        commands,
        waveform_ready,
        analysis_ready,
    } = ports;
    let session_plan = feature.session_plan().cloned().map(|plan| {
        if mode == CaptureStartMode::CaptureNow {
            plan.capture_now()
        } else {
            plan
        }
    });
    let triggered_recording = session_plan
        .as_ref()
        .map(|plan| plan.policy.effective.start == RecordingStart::Trigger)
        .unwrap_or_else(|| feature.has_trigger_program());
    let host_enforces_completion =
        feature.capabilities().data_delivery() == CaptureDataDelivery::DuringAcquisition;
    let recording_gate = if triggered_recording {
        CaptureRecordingGate::pending()
    } else {
        CaptureRecordingGate::immediate()
    };
    let descriptor =
        CaptureStoreDescriptor::new(session_id, feature.channels().to_vec()).map_err(|error| {
            CaptureCoordinatorError::store(CaptureStoreOperation::CreateStore, error)
        })?;
    let (store, writer) = CaptureStore::create(CaptureStoreConfig::new(
        repository.artifact_repository(),
        descriptor,
    ))
    .map_err(|error| CaptureCoordinatorError::store(CaptureStoreOperation::CreateStore, error))?;
    let timeline =
        CaptureTimelineMetadata::new(feature.sample_rate_hz(), feature.channel_names().to_vec())
            .map_err(|error| {
                CaptureCoordinatorError::store(CaptureStoreOperation::WriteTimelineMetadata, error)
            })?;
    store.write_timeline_metadata(timeline).map_err(|error| {
        CaptureCoordinatorError::store(CaptureStoreOperation::WriteTimelineMetadata, error)
    })?;
    let graph_source_factory = feature.graph_source_factory();
    let sample_rate_hz = feature.sample_rate_hz();
    let analysis_cursor = store.open_cursor().map_err(|error| {
        CaptureCoordinatorError::store(CaptureStoreOperation::OpenAnalysisCursor, error)
    })?;
    let analysis_cursor = recording_gate.cursor(Box::new(analysis_cursor));
    let analysis_process = graph_source_factory
        .create(Box::new(analysis_cursor))
        .map_err(|error| {
            CaptureCoordinatorError::graph_source(CaptureAttachmentKind::LiveAnalysis, error)
        })?;
    analysis_ready
        .send(CaptureAnalysisAttachment {
            source_node: feature.source_node(),
            process: analysis_process,
        })
        .map_err(|_| {
            CaptureCoordinatorError::protocol("live analysis attachment receiver closed")
        })?;
    let source_node = feature.source_node();
    let source_title = feature.source_title().to_owned();
    let (waveform, waveform_worker) = GrowingCaptureIndex::spawn(
        store.clone(),
        source_title,
        feature.sample_rate_hz(),
        feature.channel_names().to_vec(),
        Arc::clone(&work_executor),
    )
    .map_err(|error| CaptureCoordinatorError::waveform(CaptureWaveformOperation::Build, error))?;
    let mut waveform_published = false;
    let runtime = Arc::new(Mutex::new(CaptureRuntimeSignals::default()));
    let events = RecordingEventPublisher {
        inner: events,
        recording_gate: recording_gate.clone(),
        waveform: waveform.clone(),
        store: store.clone(),
        runtime: Arc::clone(&runtime),
        last_health_at: Instant::now(),
        last_health_bytes: 0,
    };
    let context = AcquisitionContext::new(session_id, Box::new(writer), Box::new(events))
        .with_work_executor(Arc::clone(&work_executor));
    let mut acquisition = feature
        .prepare(context, mode)
        .map_err(CaptureCoordinatorError::Acquisition)?;
    acquisition
        .start()
        .map_err(CaptureCoordinatorError::Acquisition)?;

    let mut stop_requested = false;
    let mut abort_requested = false;
    let mut trigger_timeout = session_plan
        .as_ref()
        .and_then(|plan| plan.policy.effective.trigger_timeout)
        .map(|timeout| (Instant::now() + timeout.after, timeout.action));
    while !acquisition.is_finished() {
        match commands.recv_timeout(SUPERVISOR_POLL_INTERVAL) {
            Ok(CaptureCommand::Stop) if !stop_requested => {
                stop_requested = true;
                acquisition
                    .request_stop()
                    .map_err(CaptureCoordinatorError::Acquisition)?;
            }
            Ok(CaptureCommand::Abort) if !abort_requested => {
                abort_requested = true;
                acquisition
                    .request_abort()
                    .map_err(CaptureCoordinatorError::Acquisition)?;
            }
            Ok(CaptureCommand::ForceTrigger) => {
                acquisition
                    .request_force_trigger()
                    .map_err(CaptureCoordinatorError::Acquisition)?;
            }
            Ok(CaptureCommand::PrepareConfigurationEpoch { graph, response }) => {
                let result = prepare_configuration_epoch(
                    &mut application_metadata,
                    repository.artifact_repository().as_ref(),
                    session_id,
                    *graph,
                    &store,
                    &recording_gate,
                    sample_rate_hz,
                );
                let _ = response.send(result);
            }
            Ok(CaptureCommand::ResolveConfigurationEpoch {
                epoch_id,
                resolution,
                response,
            }) => {
                let result = resolve_configuration_epoch(
                    &mut application_metadata,
                    repository.artifact_repository().as_ref(),
                    session_id,
                    epoch_id,
                    resolution,
                );
                let _ = response.send(result);
            }
            Ok(CaptureCommand::Stop | CaptureCommand::Abort)
            | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) if !stop_requested => {
                stop_requested = true;
                acquisition
                    .request_stop()
                    .map_err(CaptureCoordinatorError::Acquisition)?;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {}
        }

        let signals = runtime.lock().unwrap_or_else(|error| error.into_inner());
        let trigger_sample = signals.trigger_sample;
        let captured_samples = signals.captured_samples;
        drop(signals);
        if trigger_sample.is_some() {
            trigger_timeout = None;
        }
        if !stop_requested
            && let Some((deadline, action)) = trigger_timeout
            && Instant::now() >= deadline
        {
            trigger_timeout = None;
            match action {
                TriggerTimeoutAction::ContinueWaiting => {}
                TriggerTimeoutAction::Stop => {
                    stop_requested = true;
                    acquisition
                        .request_stop()
                        .map_err(CaptureCoordinatorError::Acquisition)?;
                }
                TriggerTimeoutAction::ForceTrigger => {
                    acquisition
                        .request_force_trigger()
                        .map_err(CaptureCoordinatorError::Acquisition)?;
                }
            }
        }
        let origin = trigger_sample.or((!triggered_recording).then_some(0));
        if host_enforces_completion
            && !stop_requested
            && let (Some(plan), Some(origin)) = (&session_plan, origin)
            && let Some(completion) = plan
                .policy
                .completion_sample(origin, plan.sample_rate_hz)
                .map_err(CaptureCoordinatorError::CapturePolicy)?
            && captured_samples >= completion
        {
            stop_requested = true;
            acquisition
                .request_stop()
                .map_err(CaptureCoordinatorError::Acquisition)?;
        }
        let waveform_metadata = waveform.current_metadata();
        if !waveform_published
            && waveform_ready_for_publication(
                triggered_recording,
                trigger_sample,
                waveform_metadata.total_samples,
                store.snapshot().committed_chunks != 0,
            )
        {
            let _ = waveform_ready.send(waveform.clone());
            waveform_published = true;
        }
    }
    let outcome = acquisition
        .join()
        .map_err(CaptureCoordinatorError::Acquisition)?;
    let resolution_deadline = Instant::now() + Duration::from_millis(500);
    while application_metadata.as_ref().is_some_and(|metadata| {
        metadata
            .configuration_epochs
            .iter()
            .any(|epoch| epoch.outcome == PersistedConfigurationEpochOutcome::Pending)
    }) && Instant::now() < resolution_deadline
    {
        match commands.recv_timeout(SUPERVISOR_POLL_INTERVAL) {
            Ok(CaptureCommand::ResolveConfigurationEpoch {
                epoch_id,
                resolution,
                response,
            }) => {
                let result = resolve_configuration_epoch(
                    &mut application_metadata,
                    repository.artifact_repository().as_ref(),
                    session_id,
                    epoch_id,
                    resolution,
                );
                let _ = response.send(result);
            }
            Ok(CaptureCommand::PrepareConfigurationEpoch { response, .. }) => {
                let _ = response.send(Err(CaptureCoordinatorError::protocol(
                    "capture ended before the configuration epoch was prepared",
                )));
            }
            Ok(CaptureCommand::Stop | CaptureCommand::Abort | CaptureCommand::ForceTrigger) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    while let Ok(command) = commands.try_recv() {
        match command {
            CaptureCommand::ResolveConfigurationEpoch {
                epoch_id,
                resolution,
                response,
            } => {
                let result = resolve_configuration_epoch(
                    &mut application_metadata,
                    repository.artifact_repository().as_ref(),
                    session_id,
                    epoch_id,
                    resolution,
                );
                let _ = response.send(result);
            }
            CaptureCommand::PrepareConfigurationEpoch { response, .. } => {
                let _ = response.send(Err(CaptureCoordinatorError::protocol(
                    "capture ended before the configuration epoch was prepared",
                )));
            }
            CaptureCommand::Stop | CaptureCommand::Abort | CaptureCommand::ForceTrigger => {}
        }
    }
    if !waveform_published {
        let _ = waveform_ready.send(waveform.clone());
    }
    if !recording_gate.is_resolved() {
        recording_gate.finish_without_trigger();
    }
    waveform_worker.join().map_err(|error| {
        CaptureCoordinatorError::waveform(CaptureWaveformOperation::Build, error)
    })?;
    let session_plan = session_plan.map(|plan| match recording_gate.recording_origin() {
        Some(sample) => plan.with_actual_trigger_sample(sample),
        None => plan,
    });
    if let Some(plan) = &session_plan {
        store.write_session_plan(plan).map_err(|error| {
            CaptureCoordinatorError::store(CaptureStoreOperation::WriteSessionPlan, error)
        })?;
    }
    let session_outcome = match outcome.completion {
        CaptureCompletion::Finished => CaptureSessionOutcome::Complete,
        CaptureCompletion::Stopped => CaptureSessionOutcome::Stopped,
        CaptureCompletion::CancelledBeforeTrigger => CaptureSessionOutcome::CancelledBeforeTrigger,
        CaptureCompletion::Aborted => CaptureSessionOutcome::Aborted,
    };
    let trigger_sample = runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .trigger_sample;
    let capture = store
        .finalize_with_details(
            session_outcome,
            recording_gate.recording_origin(),
            trigger_sample,
        )
        .map_err(|error| {
            CaptureCoordinatorError::store(CaptureStoreOperation::FinalizeSession, error)
        })?;
    Ok(PublishedCapture {
        _session_pin: session_pin,
        capture,
        waveform,
        source_node,
        graph_source_factory,
        recording_origin: recording_gate.recording_origin(),
        session_plan,
        outcome: session_outcome,
        completion: Some(outcome.completion),
        waveform_worker: None,
    })
}

pub(crate) fn waveform_ready_for_publication(
    triggered_recording: bool,
    trigger_sample: Option<u64>,
    indexed_samples: u64,
    has_written_chunks: bool,
) -> bool {
    if !has_written_chunks {
        return false;
    }
    if !triggered_recording {
        return true;
    }
    trigger_sample.is_some_and(|sample| indexed_samples > sample)
}
