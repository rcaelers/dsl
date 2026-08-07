use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use logic_analyzer_graph_capabilities::node::{CaptureGraphSourceFactory, LiveCaptureFeature};
use logic_analyzer_graph_capabilities::node_support::SimpleTriggerChannel;
use logic_analyzer_graph_compiler::DiscoveredLiveCaptureFeature;
use logic_analyzer_trigger::SimpleTriggerCondition;
use node_graph::NodeId;
use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository};
use platform_runtime::WorkExecutor;
use signal_capture::CaptureChannelId;
use signal_capture_session::{
    AcquisitionContext, AcquisitionError, AcquisitionResult, CaptureAnalysisChannel,
    CaptureAnalysisSource, CaptureCommandCapabilities, CaptureDataDelivery, CapturePolicy,
    CaptureProviderCapabilities, CaptureSessionPlan, CaptureSessionState, CaptureStartMode,
    CaptureStoreCursor, CompletionPolicy, EffectiveCapturePolicy, PreparedAcquisition,
    RecordingStart, RetentionPolicy, TriggerTimeout, TriggerTimeoutAction,
};
use signal_runtime::ProcessNode;

use super::super::test_acquisition_tests::{
    BufferedFakeConfig, BufferedFakeController, BufferedFakeProvider, DeterministicFakeConfig,
    DeterministicFakeController, DeterministicFakeProvider,
};
use super::{
    CaptureCoordinator, CaptureCoordinatorContract, CaptureRawExportFormat, WorkerCompletion,
    waveform_ready_for_publication,
};
use crate::capture_export_service::unavailable_capture_export_service;

struct TestWorkExecutor;

impl WorkExecutor for TestWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn supports_long_running_tasks(&self) -> bool {
        true
    }

    fn submit(
        &self,
        task: platform_runtime::WorkExecutorTask,
    ) -> Result<Box<dyn platform_runtime::WorkTask>, platform_runtime::WorkExecutorError> {
        std::thread::spawn(task);
        Ok(Box::new(platform_runtime::CompletedWorkTask))
    }
}

pub(crate) fn test_work_executor() -> Arc<dyn WorkExecutor> {
    Arc::new(TestWorkExecutor)
}

impl CaptureCoordinator {
    fn start(
        &mut self,
        feature: DiscoveredLiveCaptureFeature,
        mode: CaptureStartMode,
    ) -> Result<(), String> {
        self.start_session(feature, None, mode)
    }
}

#[test]
fn configured_coordinator_retains_the_injected_artifact_repository() {
    let artifacts: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
    let coordinator = CaptureCoordinator::configured(
        10,
        20 * 1024 * 1024 * 1024,
        Arc::clone(&artifacts),
        test_work_executor(),
        unavailable_capture_export_service(),
    );

    assert!(Arc::ptr_eq(&artifacts, &coordinator.artifact_repository()));
}

#[test]
fn triggered_waveform_is_published_only_with_its_complete_trigger_prefix() {
    assert!(!waveform_ready_for_publication(true, None, 200, true));
    assert!(!waveform_ready_for_publication(true, Some(110), 110, true));
    assert!(waveform_ready_for_publication(true, Some(110), 111, true));
    assert!(!waveform_ready_for_publication(true, Some(0), 1, false));

    assert!(waveform_ready_for_publication(false, None, 0, true));
}

type PrepareCapture =
    Box<dyn FnOnce(AcquisitionContext) -> AcquisitionResult<Box<dyn PreparedAcquisition>> + Send>;

struct FakeFeature {
    channels: Vec<CaptureChannelId>,
    channel_names: Vec<String>,
    sample_rate_hz: f64,
    prepare: Option<PrepareCapture>,
    prepare_calls: Arc<AtomicUsize>,
    simple_trigger_channels: Vec<SimpleTriggerChannel>,
    capabilities: CaptureProviderCapabilities,
    session_plan: Option<CaptureSessionPlan>,
}

struct TestGraphSourceFactory {
    channels: Vec<CaptureChannelId>,
    sample_rate_hz: f64,
}

impl CaptureGraphSourceFactory for TestGraphSourceFactory {
    fn create(&self, cursor: Box<dyn CaptureStoreCursor>) -> Result<Box<dyn ProcessNode>, String> {
        test_analysis_source(&self.channels, self.sample_rate_hz, cursor)
    }
}

impl LiveCaptureFeature for FakeFeature {
    fn channels(&self) -> &[CaptureChannelId] {
        &self.channels
    }

    fn channel_names(&self) -> &[String] {
        &self.channel_names
    }

    fn sample_rate_hz(&self) -> f64 {
        self.sample_rate_hz
    }

    fn capabilities(&self) -> &CaptureProviderCapabilities {
        &self.capabilities
    }

    fn simple_trigger_channels(&self) -> &[SimpleTriggerChannel] {
        &self.simple_trigger_channels
    }

    fn session_plan(&self) -> Option<&CaptureSessionPlan> {
        self.session_plan.as_ref()
    }

    fn graph_source_factory(&self) -> Arc<dyn CaptureGraphSourceFactory> {
        Arc::new(TestGraphSourceFactory {
            channels: self.channels.clone(),
            sample_rate_hz: self.sample_rate_hz,
        })
    }

    fn prepare(
        self: Box<Self>,
        context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        let mut feature = *self;
        feature.prepare_calls.fetch_add(1, Ordering::SeqCst);
        feature
            .prepare
            .take()
            .expect("test live-capture feature prepares at most once")(context)
    }

    fn prepare_with_mode(
        self: Box<Self>,
        context: AcquisitionContext,
        _mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        self.prepare(context)
    }
}

struct FailingFeature {
    channels: Vec<CaptureChannelId>,
    channel_names: Vec<String>,
    capabilities: CaptureProviderCapabilities,
}

impl LiveCaptureFeature for FailingFeature {
    fn channels(&self) -> &[CaptureChannelId] {
        &self.channels
    }

    fn channel_names(&self) -> &[String] {
        &self.channel_names
    }

    fn sample_rate_hz(&self) -> f64 {
        1_000_000_000.0
    }

    fn capabilities(&self) -> &CaptureProviderCapabilities {
        &self.capabilities
    }

    fn graph_source_factory(&self) -> Arc<dyn CaptureGraphSourceFactory> {
        Arc::new(TestGraphSourceFactory {
            channels: self.channels.clone(),
            sample_rate_hz: self.sample_rate_hz(),
        })
    }

    fn prepare(
        self: Box<Self>,
        _context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        Err(AcquisitionError::InvalidRequest(
            "intentional preparation failure".into(),
        ))
    }
}

fn test_analysis_source(
    channels: &[CaptureChannelId],
    sample_rate_hz: f64,
    cursor: Box<dyn CaptureStoreCursor>,
) -> Result<Box<dyn ProcessNode>, String> {
    let layout = channels
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, channel)| CaptureAnalysisChannel::polymorphic(channel, format!("ch{index}")))
        .collect();
    CaptureAnalysisSource::new("test-live-analysis", cursor, sample_rate_hz, layout)
        .map(|source| Box::new(source) as Box<dyn ProcessNode>)
}

fn streaming_capabilities(channels: &[CaptureChannelId]) -> CaptureProviderCapabilities {
    CaptureProviderCapabilities::single(
        CaptureDataDelivery::DuringAcquisition,
        channels.to_vec(),
        1_000_000_000,
    )
    .with_commands(CaptureCommandCapabilities::new(true, true, true, true))
}

fn manual_feature_with_samples(
    sample_counts: Vec<u64>,
) -> (
    DiscoveredLiveCaptureFeature,
    DeterministicFakeController,
    Arc<AtomicUsize>,
) {
    let channels = vec![
        CaptureChannelId::new("bank-a:7"),
        CaptureChannelId::new("bank-c:2"),
    ];
    let config = DeterministicFakeConfig::new(channels.clone(), sample_counts, 0x5a17).unwrap();
    let (provider, controller) = DeterministicFakeProvider::manually_paced(config);
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let capabilities = streaming_capabilities(&channels);
    let feature = DiscoveredLiveCaptureFeature::new(
        NodeId(41),
        "Contract Fake",
        Box::new(FakeFeature {
            channel_names: vec!["Bank A 7".into(), "Bank C 2".into()],
            channels,
            sample_rate_hz: 1_000_000_000.0,
            prepare: Some(Box::new(move |context| provider.prepare(context))),
            prepare_calls: Arc::clone(&prepare_calls),
            simple_trigger_channels: Vec::new(),
            capabilities,
            session_plan: None,
        }),
    );
    (feature, controller, prepare_calls)
}

fn manual_feature_with_counter() -> (
    DiscoveredLiveCaptureFeature,
    DeterministicFakeController,
    Arc<AtomicUsize>,
) {
    manual_feature_with_samples(vec![3, 5, 2, 7])
}

fn manual_feature() -> (DiscoveredLiveCaptureFeature, DeterministicFakeController) {
    let (feature, controller, _) = manual_feature_with_counter();
    (feature, controller)
}

fn manual_triggered_feature_with_timeout_and_counter(
    trigger_timeout: Option<TriggerTimeout>,
) -> (
    DiscoveredLiveCaptureFeature,
    DeterministicFakeController,
    u64,
    Arc<AtomicUsize>,
) {
    let channels = (0..19)
        .map(|channel| {
            CaptureChannelId::new(format!("stream-bank-{}:{}", channel % 4, channel * 7 + 3))
        })
        .collect::<Vec<_>>();
    let mut trigger_conditions = vec![None; channels.len()];
    trigger_conditions[0] = Some(SimpleTriggerCondition::Rising);
    let config = DeterministicFakeConfig::new(channels.clone(), vec![3, 5, 2, 7], 0x5a17)
        .unwrap()
        .with_simple_trigger(trigger_conditions)
        .unwrap();
    let trigger_sample = config.first_trigger_sample().unwrap();
    let total_samples = config.total_samples();
    let (provider, controller) = DeterministicFakeProvider::manually_paced(config);
    let capabilities = streaming_capabilities(&channels);
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let feature = DiscoveredLiveCaptureFeature::new(
        NodeId(42),
        "Triggered Contract Fake",
        Box::new(FakeFeature {
            channel_names: (0..channels.len())
                .map(|channel| format!("Streaming {channel}"))
                .collect(),
            simple_trigger_channels: vec![SimpleTriggerChannel {
                channel_id: channels[0].clone(),
                viewer_channel: 0,
                name: "Streaming 0".into(),
                enabled: true,
                condition: SimpleTriggerCondition::Rising,
            }],
            channels,
            sample_rate_hz: 1_000_000_000.0,
            prepare: Some(Box::new(move |context| provider.prepare(context))),
            prepare_calls: Arc::clone(&prepare_calls),
            capabilities,
            session_plan: Some(CaptureSessionPlan {
                sample_rate_hz: 1_000_000_000,
                channel_count: 19,
                capture_window_samples: Some(total_samples),
                policy: EffectiveCapturePolicy {
                    requested: CapturePolicy {
                        start: RecordingStart::Trigger,
                        trigger_placement: None,
                        retention_before_origin: RetentionPolicy::Everything,
                        retention_after_origin: RetentionPolicy::Everything,
                        completion: CompletionPolicy::UntilStopped,
                        trigger_timeout,
                    },
                    effective: CapturePolicy {
                        start: RecordingStart::Trigger,
                        trigger_placement: None,
                        retention_before_origin: RetentionPolicy::Everything,
                        retention_after_origin: RetentionPolicy::Everything,
                        completion: CompletionPolicy::UntilStopped,
                        trigger_timeout,
                    },
                },
            }),
        }),
    );
    (feature, controller, trigger_sample, prepare_calls)
}

fn manual_triggered_feature_with_counter() -> (
    DiscoveredLiveCaptureFeature,
    DeterministicFakeController,
    u64,
    Arc<AtomicUsize>,
) {
    manual_triggered_feature_with_timeout_and_counter(None)
}

fn manual_triggered_feature() -> (
    DiscoveredLiveCaptureFeature,
    DeterministicFakeController,
    u64,
) {
    let (feature, controller, trigger_sample, _) = manual_triggered_feature_with_counter();
    (feature, controller, trigger_sample)
}

fn manual_capture_now_feature() -> (DiscoveredLiveCaptureFeature, DeterministicFakeController) {
    let channels = vec![CaptureChannelId::new("capture-now:0")];
    let config = DeterministicFakeConfig::new(channels.clone(), vec![3, 5, 2, 7], 0x5a17).unwrap();
    let total_samples = config.total_samples();
    let (provider, controller) = DeterministicFakeProvider::manually_paced(config);
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let feature = DiscoveredLiveCaptureFeature::new(
        NodeId(44),
        "Capture Now Contract Fake",
        Box::new(FakeFeature {
            channel_names: vec!["Capture Now 0".into()],
            simple_trigger_channels: vec![SimpleTriggerChannel {
                channel_id: channels[0].clone(),
                viewer_channel: 0,
                name: "Capture Now 0".into(),
                enabled: true,
                condition: SimpleTriggerCondition::Rising,
            }],
            channels: channels.clone(),
            sample_rate_hz: 1_000_000_000.0,
            prepare: Some(Box::new(move |context| provider.prepare(context))),
            prepare_calls,
            capabilities: streaming_capabilities(&channels),
            session_plan: Some(CaptureSessionPlan {
                sample_rate_hz: 1_000_000_000,
                channel_count: 1,
                capture_window_samples: Some(total_samples),
                policy: EffectiveCapturePolicy {
                    requested: CapturePolicy {
                        start: RecordingStart::Trigger,
                        trigger_placement: None,
                        retention_before_origin: RetentionPolicy::Everything,
                        retention_after_origin: RetentionPolicy::Everything,
                        completion: CompletionPolicy::UntilStopped,
                        trigger_timeout: None,
                    },
                    effective: CapturePolicy {
                        start: RecordingStart::Trigger,
                        trigger_placement: None,
                        retention_before_origin: RetentionPolicy::Everything,
                        retention_after_origin: RetentionPolicy::Everything,
                        completion: CompletionPolicy::UntilStopped,
                        trigger_timeout: None,
                    },
                },
            }),
        }),
    );
    (feature, controller)
}

fn buffered_triggered_feature() -> (
    DiscoveredLiveCaptureFeature,
    BufferedFakeController,
    u64,
    Arc<AtomicUsize>,
) {
    let channels = vec![
        CaptureChannelId::new("pod-a:3"),
        CaptureChannelId::new("pod-q:41"),
        CaptureChannelId::new("aux-bank:9"),
    ];
    let sample_rate_hz = 2_000_000_u64;
    let config = BufferedFakeConfig::new(channels.clone(), sample_rate_hz, 19, 5, 0x8d31)
        .unwrap()
        .with_simple_trigger(vec![None, Some(SimpleTriggerCondition::Falling), None])
        .unwrap();
    let trigger_sample = config.first_trigger_sample().unwrap();
    let capabilities = config.capabilities().clone();
    let (provider, controller) = BufferedFakeProvider::manually_uploaded(config);
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let policy = CapturePolicy {
        start: RecordingStart::Trigger,
        trigger_placement: None,
        retention_before_origin: RetentionPolicy::Everything,
        retention_after_origin: RetentionPolicy::Everything,
        completion: CompletionPolicy::SamplesAfterOrigin(1),
        trigger_timeout: None,
    };
    let feature = DiscoveredLiveCaptureFeature::new(
        NodeId(43),
        "Buffered Contract Fake",
        Box::new(FakeFeature {
            channel_names: vec!["Pod A 3".into(), "Pod Q 41".into(), "Aux 9".into()],
            simple_trigger_channels: vec![SimpleTriggerChannel {
                channel_id: channels[1].clone(),
                viewer_channel: 1,
                name: "Pod Q 41".into(),
                enabled: true,
                condition: SimpleTriggerCondition::Falling,
            }],
            channels,
            sample_rate_hz: sample_rate_hz as f64,
            prepare: Some(Box::new(move |context| provider.prepare(context))),
            prepare_calls: Arc::clone(&prepare_calls),
            capabilities,
            session_plan: Some(CaptureSessionPlan {
                sample_rate_hz,
                channel_count: 3,
                capture_window_samples: Some(21),
                policy: EffectiveCapturePolicy {
                    requested: policy.clone(),
                    effective: policy,
                },
            }),
        }),
    );
    (feature, controller, trigger_sample, prepare_calls)
}

fn poll_until(
    coordinator: &mut CaptureCoordinator,
    condition: impl Fn(&CaptureCoordinator) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !condition(coordinator) {
        assert!(Instant::now() < deadline, "capture coordinator timed out");
        coordinator.poll();
        std::thread::yield_now();
    }
}

#[test]
fn failed_capture_does_not_detach_an_already_published_waveform() {
    let mut coordinator = CaptureCoordinator::new();

    coordinator.finish_worker(WorkerCompletion::Failed("stream overflow".into()));

    assert!(coordinator.take_waveform_update().is_none());
}

fn run_triggered_coordinator_contract(
    feature: DiscoveredLiveCaptureFeature,
    expected_delivery: CaptureDataDelivery,
    expected_samples: u64,
    expected_trigger: u64,
    prepare_calls: Arc<AtomicUsize>,
    drive_capture: impl FnOnce(),
) {
    let source_node = feature.source_node();
    let channels = feature.channels().to_vec();
    assert_eq!(feature.capabilities().data_delivery(), expected_delivery);

    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    drive_capture();
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    let manifest = coordinator.completed_manifest().unwrap();
    assert_eq!(manifest.descriptor.channels(), channels);
    assert_eq!(manifest.committed_chunks, 4);
    assert_eq!(manifest.committed_samples, expected_samples);
    assert_eq!(
        coordinator.completed_recording_origin(),
        Some(expected_trigger)
    );
    assert_eq!(
        coordinator.completed_trigger_sample(),
        Some(expected_trigger)
    );
    assert_eq!(
        coordinator.status().unwrap().trigger_sample,
        Some(expected_trigger)
    );
    let states = coordinator.state_history();
    assert!(states.contains(&CaptureSessionState::Prepared));
    assert!(states.contains(&CaptureSessionState::Armed));
    assert!(states.contains(&CaptureSessionState::Triggered));
    assert!(states.contains(&CaptureSessionState::Recording));
    assert_eq!(states.last(), Some(&CaptureSessionState::Complete));

    let waveform = coordinator
        .take_waveform_update()
        .expect("coordinator should publish a waveform update")
        .expect("completed capture should retain its waveform");
    let mut viewer = logic_analyzer_viewer::LogicAnalyzerViewer::new();
    viewer.set_growing_capture(waveform);
    assert!(viewer.has_growing_capture());
    assert!(viewer.growing_capture_complete());

    let analysis = coordinator
        .take_analysis_attachment()
        .expect("coordinator should attach live analysis");
    assert_eq!(analysis.source_node, source_node);
    let analysis_schema = analysis
        .process
        .output_schema()
        .into_iter()
        .map(|port| (port.name, port.type_id, port.index, port.payloads))
        .collect::<Vec<_>>();
    assert_eq!(analysis_schema.len(), channels.len());

    let first_replay = coordinator
        .create_replay_attachment()
        .unwrap()
        .expect("completed capture should be replayable");
    let second_replay = coordinator
        .create_replay_attachment()
        .unwrap()
        .expect("every replay should receive a fresh cursor");
    assert_eq!(first_replay.source_node, source_node);
    assert_eq!(second_replay.source_node, source_node);
    let replay_schema = |process: &dyn ProcessNode| {
        process
            .output_schema()
            .into_iter()
            .map(|port| (port.name, port.type_id, port.index, port.payloads))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        analysis_schema,
        replay_schema(first_replay.process.as_ref())
    );
    assert_eq!(
        analysis_schema,
        replay_schema(second_replay.process.as_ref())
    );
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn finalized_capture_routes_export_through_the_injected_service() {
    let (feature, controller) = manual_feature();
    let (mut coordinator, export_control) = CaptureCoordinator::new_with_scripted_export();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    let output = PathBuf::from("background.sr");
    let session_id = coordinator.current_session_id().unwrap();
    coordinator
        .start_export_current(CaptureRawExportFormat::Portable, output.clone())
        .unwrap();
    poll_until(&mut coordinator, |coordinator| {
        coordinator.export_status().is_none()
    });
    let completion = coordinator.take_export_notice().unwrap().unwrap();
    assert_eq!(completion.destination, output);
    assert!(completion.warnings.is_empty());
    assert_eq!(
        export_control.starts(),
        vec![(session_id, CaptureRawExportFormat::Portable, output)]
    );
}

#[test]
fn streaming_and_buffered_profiles_share_the_coordinator_contract() {
    let (feature, controller, trigger_sample, prepare_calls) =
        manual_triggered_feature_with_counter();
    run_triggered_coordinator_contract(
        feature,
        CaptureDataDelivery::DuringAcquisition,
        17,
        trigger_sample,
        prepare_calls,
        move || controller.grant_chunks(4),
    );

    let (feature, controller, trigger_sample, prepare_calls) = buffered_triggered_feature();
    run_triggered_coordinator_contract(
        feature,
        CaptureDataDelivery::BufferedUpload,
        19,
        trigger_sample,
        prepare_calls,
        move || {
            assert!(controller.wait_until_upload(Duration::from_secs(2)));
            controller.grant_upload_chunks(4);
        },
    );
}

#[test]
fn raw_only_capture_completes_after_its_analysis_attachment_is_dropped() {
    let (feature, controller) = manual_feature();

    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    controller.grant_chunks(4);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        coordinator.poll();
        if coordinator.take_analysis_attachment().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "analysis attachment was not delivered"
        );
        std::thread::yield_now();
    }
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert_eq!(
        coordinator.status().unwrap().state,
        CaptureSessionState::Complete,
        "{:?}",
        coordinator.status().unwrap().error
    );
}

#[test]
fn starting_a_new_capture_discards_the_previous_store_and_index() {
    let graph = node_graph::GraphState::default();
    let (feature, first_controller) = manual_feature();
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start_with_graph(feature, &graph, CaptureStartMode::SavedPolicy)
        .unwrap();
    first_controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());
    let first_session = coordinator.current_session_id().unwrap();
    assert!(coordinator.capture_session_exists(first_session));

    let (feature, second_controller) = manual_feature();
    coordinator
        .start_with_graph(feature, &graph, CaptureStartMode::SavedPolicy)
        .unwrap();
    assert!(!coordinator.capture_session_exists(first_session));
    second_controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());
    assert_ne!(coordinator.current_session_id(), Some(first_session));
}

#[test]
fn immediate_capture_uses_commands_and_restores_editing_after_finalization() {
    let (feature, controller) = manual_feature();
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    assert!(!coordinator.graph_editing_enabled());

    controller.grant_chunks(2);
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.progress.captured_samples == Some(8))
    });
    coordinator.request_stop();
    coordinator.request_stop();
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert!(coordinator.graph_editing_enabled());
    assert_eq!(
        coordinator.state_history(),
        [
            CaptureSessionState::Preparing,
            CaptureSessionState::Prepared,
            CaptureSessionState::Recording,
            CaptureSessionState::Stopping,
            CaptureSessionState::Complete,
        ]
    );
    let manifest = coordinator.completed_manifest().unwrap();
    assert_eq!(
        manifest.descriptor.session_id(),
        coordinator.status().unwrap().session_id
    );
    assert_eq!(manifest.committed_chunks, 2);
    assert_eq!(manifest.committed_samples, 8);
}

#[test]
fn configuration_epoch_is_persisted_before_runtime_application_and_resolved() {
    let (feature, controller) = manual_feature();
    let mut coordinator = CaptureCoordinator::new();
    let graph = node_graph::GraphState::default();
    coordinator
        .start_with_graph(feature, &graph, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.state == CaptureSessionState::Recording)
    });
    assert!(coordinator.graph_editing_enabled());
    controller.grant_chunks(2);
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .and_then(|status| status.progress.captured_samples)
            == Some(8)
    });

    let mut edited = graph.clone();
    edited
        .set_extension("test.configuration_epoch", 1_u64)
        .unwrap();
    coordinator.request_configuration_epoch(edited).unwrap();
    let prepared = loop {
        coordinator.poll();
        if let Some(result) = coordinator.take_configuration_epoch_preparation() {
            break result.unwrap();
        }
        std::thread::yield_now();
    };
    assert_eq!(prepared.epoch_id, 1);
    // Every accepted capture chunk advances the shared committed frontier,
    // so epoch boundaries use the same sample visible to replay and viewers.
    assert_eq!(prepared.source_sample, 8);
    assert_eq!(prepared.boundary.sample_index, 8);
    coordinator
        .resolve_configuration_epoch(
            prepared.epoch_id,
            super::super::implementation::ConfigurationEpochResolution::Applied,
        )
        .unwrap();
    loop {
        coordinator.poll();
        if let Some(result) = coordinator.take_configuration_epoch_notice() {
            result.unwrap();
            break;
        }
        std::thread::yield_now();
    }
    coordinator.request_stop();
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    let session_id = coordinator.current_session_id().unwrap();
    let artifacts = coordinator.artifact_repository();
    let metadata = super::read_application_metadata(artifacts.as_ref(), session_id).unwrap();
    assert_eq!(metadata.configuration_epochs.len(), 1);
    let epoch = &metadata.configuration_epochs[0];
    assert_eq!(epoch.source_sample, prepared.source_sample);
    assert_eq!(epoch.analysis_sample, prepared.boundary.sample_index);
    assert_eq!(epoch.timestamp_ns, prepared.boundary.timestamp_ns);
    assert_eq!(
        epoch.outcome,
        super::PersistedConfigurationEpochOutcome::Applied
    );
}

#[test]
fn interrupted_pending_configuration_epoch_recovers_as_failed() {
    let (feature, controller) = manual_feature();
    let mut coordinator = CaptureCoordinator::new();
    let graph = node_graph::GraphState::default();
    coordinator
        .start_with_graph(feature, &graph, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.state == CaptureSessionState::Recording)
    });
    controller.grant_chunks(1);
    let mut edited = graph.clone();
    edited
        .set_extension("test.configuration_epoch", 2_u64)
        .unwrap();
    coordinator.request_configuration_epoch(edited).unwrap();
    loop {
        coordinator.poll();
        if coordinator.take_configuration_epoch_preparation().is_some() {
            break;
        }
        std::thread::yield_now();
    }
    coordinator.request_stop();
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    let session_id = coordinator.current_session_id().unwrap();
    let artifacts = coordinator.artifact_repository();
    let metadata = super::read_application_metadata(artifacts.as_ref(), session_id).unwrap();
    let epoch = &metadata.configuration_epochs[0];
    assert_eq!(
        epoch.outcome,
        super::PersistedConfigurationEpochOutcome::Failed
    );
    assert!(epoch.message.as_deref().unwrap().contains("before"));
}

#[test]
fn force_trigger_uses_only_the_advertised_provider_operation() {
    let (feature, controller, _natural_trigger) = manual_triggered_feature();
    assert!(feature.capabilities().commands().force_trigger);
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.state == CaptureSessionState::Armed)
    });

    coordinator.request_force_trigger().unwrap();
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.trigger_sample == Some(0))
    });
    controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert_eq!(coordinator.completed_recording_origin(), Some(0));
    assert_eq!(coordinator.completed_trigger_sample(), Some(0));
}

#[test]
fn trigger_timeout_actions_continue_stop_and_force_through_capabilities() {
    let timeout = Duration::from_millis(20);

    let (feature, _controller, _, _) =
        manual_triggered_feature_with_timeout_and_counter(Some(TriggerTimeout {
            after: timeout,
            action: TriggerTimeoutAction::Stop,
        }));
    let mut stopped = CaptureCoordinator::new();
    stopped
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut stopped, |coordinator| !coordinator.is_active());
    assert_eq!(
        stopped.status().unwrap().completion,
        Some(signal_capture_session::CaptureCompletion::CancelledBeforeTrigger)
    );
    assert_eq!(stopped.completed_manifest().unwrap().committed_samples, 0);

    let (feature, controller, _, _) =
        manual_triggered_feature_with_timeout_and_counter(Some(TriggerTimeout {
            after: timeout,
            action: TriggerTimeoutAction::ForceTrigger,
        }));
    let mut forced = CaptureCoordinator::new();
    forced
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut forced, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.trigger_sample == Some(0))
    });
    controller.grant_chunks(4);
    poll_until(&mut forced, |coordinator| !coordinator.is_active());
    assert_eq!(forced.completed_recording_origin(), Some(0));

    let (feature, _controller, _, _) =
        manual_triggered_feature_with_timeout_and_counter(Some(TriggerTimeout {
            after: timeout,
            action: TriggerTimeoutAction::ContinueWaiting,
        }));
    let mut waiting = CaptureCoordinator::new();
    waiting
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut waiting, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.state == CaptureSessionState::Armed)
    });
    std::thread::sleep(timeout * 2);
    waiting.poll();
    assert!(waiting.is_active());
    assert_eq!(waiting.status().unwrap().state, CaptureSessionState::Armed);
    waiting.request_stop();
    poll_until(&mut waiting, |coordinator| !coordinator.is_active());
}

#[test]
fn abort_retains_the_valid_committed_prefix_and_labels_it_incomplete() {
    let (feature, controller) = manual_feature();
    assert!(feature.capabilities().commands().abort);
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    controller.grant_chunks(1);
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.progress.captured_samples == Some(3))
    });

    coordinator.request_abort().unwrap();
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    let manifest = coordinator.completed_manifest().unwrap();
    assert_eq!(manifest.committed_chunks, 1);
    assert_eq!(manifest.committed_samples, 3);
    assert_eq!(
        coordinator.status().unwrap().completion,
        Some(signal_capture_session::CaptureCompletion::Aborted)
    );
}

#[test]
fn health_reports_store_rate_summary_lag_and_graph_lag_without_blocking_capture() {
    let (feature, controller) = manual_feature();
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.state == CaptureSessionState::Recording)
    });
    std::thread::sleep(Duration::from_millis(110));
    controller.grant_chunks(1);
    poll_until(&mut coordinator, |coordinator| {
        coordinator.status().is_some_and(|status| {
            status.health.input_bytes_per_second.is_some()
                && status.health.summary_lag_samples.is_some()
        })
    });
    coordinator.set_graph_processed_samples(Some(1));

    let health = coordinator.status().unwrap().health;
    assert!(health.write_bytes_per_second.is_some());
    assert_eq!(health.stored_samples, Some(3));
    assert_eq!(health.graph_lag_samples, Some(2));

    coordinator.request_abort().unwrap();
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());
}

#[test]
fn capture_now_bypasses_one_session_trigger_without_mutating_requested_policy() {
    let (feature, controller) = manual_capture_now_feature();
    assert_eq!(
        feature.session_plan().unwrap().policy.requested.start,
        signal_capture_session::RecordingStart::Trigger
    );

    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::CaptureNow)
        .unwrap();
    controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert_eq!(coordinator.completed_recording_origin(), Some(0));
    assert_eq!(coordinator.completed_trigger_sample(), None);
    let plan = coordinator.completed_session_plan().unwrap();
    assert_eq!(
        plan.policy.requested.start,
        signal_capture_session::RecordingStart::Trigger
    );
    assert_eq!(
        plan.policy.effective.start,
        signal_capture_session::RecordingStart::Immediate
    );
    assert_eq!(
        coordinator.completed_persisted_session_plan().as_ref(),
        Some(plan)
    );
}

#[test]
fn preparation_failure_keeps_editing_locked_until_cleanup_returns() {
    let feature = DiscoveredLiveCaptureFeature::new(
        NodeId(99),
        "Failing Fake",
        Box::new(FailingFeature {
            channels: vec![CaptureChannelId::new("fake:0")],
            channel_names: vec!["Fake 0".into()],
            capabilities: CaptureProviderCapabilities::single(
                CaptureDataDelivery::DuringAcquisition,
                vec![CaptureChannelId::new("fake:0")],
                1_000_000_000,
            ),
        }),
    );
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();
    assert!(!coordinator.graph_editing_enabled());

    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert!(coordinator.graph_editing_enabled());
    assert_eq!(
        coordinator.status().unwrap().state,
        CaptureSessionState::Error
    );
    assert!(
        coordinator
            .status()
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("intentional preparation failure"))
    );
    assert!(coordinator.completed_manifest().is_none());
}

#[test]
fn triggered_capture_arms_marks_the_waveform_and_defines_recording_origin() {
    let (feature, controller, expected_trigger) = manual_triggered_feature();
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();

    controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert_eq!(
        coordinator.completed_recording_origin(),
        Some(expected_trigger)
    );
    assert_eq!(
        coordinator.completed_trigger_sample(),
        Some(expected_trigger)
    );
    assert_eq!(
        coordinator.status().unwrap().trigger_sample,
        Some(expected_trigger)
    );
    let states = coordinator.state_history();
    assert!(states.contains(&CaptureSessionState::Armed));
    assert!(states.contains(&CaptureSessionState::Triggered));
    assert!(states.contains(&CaptureSessionState::Recording));
    assert!(
        states
            .iter()
            .position(|state| *state == CaptureSessionState::Armed)
            < states
                .iter()
                .position(|state| *state == CaptureSessionState::Triggered)
    );
}

#[test]
fn buffered_trigger_reveals_the_marker_with_the_indexed_pretrigger_prefix() {
    let (feature, controller, trigger_sample, _) = buffered_triggered_feature();
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();

    assert!(controller.wait_until_upload(Duration::from_secs(2)));
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .is_some_and(|status| status.trigger_sample == Some(trigger_sample))
    });
    assert!(coordinator.take_waveform_update().is_none());

    let mut granted_chunks = 0;
    if trigger_sample >= 5 {
        controller.grant_upload_chunks(1);
        granted_chunks = 1;
        poll_until(&mut coordinator, |coordinator| {
            coordinator
                .status()
                .and_then(|status| status.progress.captured_samples)
                .is_some_and(|samples| samples >= 5)
        });
        assert!(coordinator.take_waveform_update().is_none());
    }

    controller.grant_upload_chunks(4 - granted_chunks);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());
    let waveform = coordinator
        .take_waveform_update()
        .expect("triggered capture should publish its waveform")
        .expect("completed triggered capture should retain its waveform");
    let metadata = waveform.current_metadata();
    assert_eq!(metadata.trigger_sample, Some(trigger_sample));
    assert!(metadata.total_samples > trigger_sample);
}

#[test]
fn buffered_upload_is_not_cut_short_by_host_completion_policy() {
    let (feature, controller, trigger_sample, _) = buffered_triggered_feature();
    assert_eq!(trigger_sample, 10);
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();

    assert!(controller.wait_until_upload(Duration::from_secs(2)));
    controller.grant_upload_chunks(3);
    poll_until(&mut coordinator, |coordinator| {
        coordinator
            .status()
            .and_then(|status| status.progress.captured_samples)
            == Some(15)
    });

    let deadline = Instant::now() + Duration::from_millis(30);
    while Instant::now() < deadline {
        coordinator.poll();
        std::thread::yield_now();
    }
    assert!(
        coordinator.is_active(),
        "the host must not stop an upload for data already captured on the device"
    );

    controller.grant_upload_chunks(1);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());
    assert_eq!(
        coordinator.completed_manifest().unwrap().committed_samples,
        19
    );
}

#[test]
fn paused_viewer_and_analysis_do_not_delay_manual_capture() {
    let (feature, controller, _) = manual_feature_with_samples(vec![4; 32]);
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();

    controller.grant_chunks(16);
    let deadline = Instant::now() + Duration::from_secs(2);
    let index = loop {
        coordinator.poll();
        if let Some(Some(index)) = coordinator.take_waveform_update() {
            break index;
        }
        assert!(Instant::now() < deadline, "waveform attachment timed out");
        std::thread::yield_now();
    };
    let mut viewer = logic_analyzer_viewer::LogicAnalyzerViewer::new();
    viewer.set_growing_capture(index);
    viewer.toggle_pause_display();
    let _paused_analysis = coordinator
        .take_analysis_attachment()
        .expect("analysis attachment should precede waveform publication");

    controller.grant_chunks(16);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());

    assert!(viewer.display_paused());
    let manifest = coordinator.completed_manifest().unwrap();
    assert_eq!(manifest.committed_chunks, 32);
    assert_eq!(manifest.committed_samples, 128);
}

#[test]
fn finalized_replay_creates_fresh_sources_without_preparing_provider_again() {
    let (feature, controller, prepare_calls) = manual_feature_with_counter();
    let mut coordinator = CaptureCoordinator::new();
    coordinator
        .start(feature, CaptureStartMode::SavedPolicy)
        .unwrap();

    controller.grant_chunks(4);
    poll_until(&mut coordinator, |coordinator| !coordinator.is_active());
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(coordinator.replay_source_node(), Some(NodeId(41)));

    let first = coordinator
        .create_replay_attachment()
        .unwrap()
        .expect("finalized session should be replayable");
    let second = coordinator
        .create_replay_attachment()
        .unwrap()
        .expect("every Run should get a fresh replay cursor");

    assert_eq!(first.source_node, NodeId(41));
    assert_eq!(second.source_node, NodeId(41));
    let schema = |process: &dyn ProcessNode| {
        process
            .output_schema()
            .into_iter()
            .map(|port| (port.name, port.type_id, port.index, port.payloads))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        schema(first.process.as_ref()),
        schema(second.process.as_ref())
    );
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
}
