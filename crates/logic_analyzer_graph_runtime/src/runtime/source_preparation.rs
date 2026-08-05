use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::CapturePresentation;
use logic_analyzer_graph_plan::DiscoveredCapturePresentation;
use signal_artifacts::{ArtifactRepository, MemoryArtifactRepository};
#[cfg(test)]
use signal_runtime::InlineWorkExecutor;
use signal_runtime::WorkExecutor;

#[cfg(test)]
use super::source_preparation_executor::InlineSourcePreparationExecutor;
use super::source_preparation_executor::{
    SourcePreparationControl, SourcePreparationExecutor, SourcePreparationTask,
    SourcePreparationTaskUpdate,
};
use super::{
    PreparedCapture, PreparedCaptureData, PreparingCapture, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationUpdate,
};

pub(crate) struct SourcePreparation {
    identity: Option<String>,
    executor: Box<dyn SourcePreparationExecutor>,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
    task: Option<Box<dyn SourcePreparationTask>>,
    control: Option<SourcePreparationControl>,
    generation: u64,
    status: SourcePreparationStatus,
}

impl SourcePreparation {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_execution(
            Box::new(InlineSourcePreparationExecutor),
            Arc::new(InlineWorkExecutor),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_executor(executor: Box<dyn SourcePreparationExecutor>) -> Self {
        Self::with_execution(executor, Arc::new(InlineWorkExecutor))
    }

    pub(crate) fn with_execution(
        executor: Box<dyn SourcePreparationExecutor>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self {
            identity: None,
            executor,
            work_executor,
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
            task: None,
            control: None,
            generation: 0,
            status: SourcePreparationStatus::Empty,
        }
    }

    pub(crate) fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.reset();
        self.artifact_repository = repository;
    }

    pub(crate) fn synchronize(
        &mut self,
        discovered: Option<DiscoveredCapturePresentation>,
    ) -> SourcePreparationUpdate {
        let Some(discovered) = discovered else {
            let changed = self.identity.take().is_some();
            self.cancel_active();
            if changed {
                self.advance_generation();
            }
            self.status = SourcePreparationStatus::Empty;
            return if changed {
                SourcePreparationUpdate::Cleared
            } else {
                SourcePreparationUpdate::Unchanged
            };
        };
        if self.identity.as_deref() != Some(discovered.identity.as_str()) {
            self.identity = Some(discovered.identity.clone());
            return self.start(discovered);
        }
        let Some(task) = &mut self.task else {
            return SourcePreparationUpdate::Unchanged;
        };
        match task.poll() {
            SourcePreparationTaskUpdate::Complete(Ok(data)) => {
                self.task = None;
                self.control = None;
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data,
                })
            }
            SourcePreparationTaskUpdate::Complete(Err(error)) => {
                self.task = None;
                self.control = None;
                self.status = SourcePreparationStatus::Failed(error.clone());
                SourcePreparationUpdate::Failed(error)
            }
            SourcePreparationTaskUpdate::Pending => {
                self.preparing_update(discovered.identity, discovered.visible_channels)
            }
            SourcePreparationTaskUpdate::Disconnected => {
                self.task = None;
                self.control = None;
                let error = "capture preparation worker stopped".to_owned();
                self.status = SourcePreparationStatus::Failed(error.clone());
                SourcePreparationUpdate::Failed(error)
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        let changed = self.identity.is_some() || self.task.is_some();
        self.cancel_active();
        self.identity = None;
        if changed {
            self.advance_generation();
        }
        self.status = SourcePreparationStatus::Empty;
    }

    /// Records a discovery failure once, cancelling any preparation superseded by it.
    pub(crate) fn fail(&mut self, error: String) -> SourcePreparationUpdate {
        let unchanged = matches!(
            &self.status,
            SourcePreparationStatus::Failed(previous) if previous == &error
        );
        let changed = self.identity.is_some() || self.task.is_some();
        self.cancel_active();
        self.identity = None;
        if changed {
            self.advance_generation();
        }
        self.status = SourcePreparationStatus::Failed(error.clone());
        if unchanged {
            SourcePreparationUpdate::Unchanged
        } else {
            SourcePreparationUpdate::Failed(error)
        }
    }

    pub(crate) fn status(&self) -> SourcePreparationStatus {
        self.status.clone()
    }

    pub(crate) fn snapshot(&self) -> SourcePreparationSnapshot {
        SourcePreparationSnapshot {
            generation: self.generation,
            status: self.status.clone(),
            progress: self
                .control
                .as_ref()
                .and_then(SourcePreparationControl::progress),
        }
    }

    fn start(&mut self, discovered: DiscoveredCapturePresentation) -> SourcePreparationUpdate {
        self.cancel_active();
        self.advance_generation();
        self.status = SourcePreparationStatus::Preparing;
        let identity = discovered.identity;
        let visible_channels = discovered.visible_channels;
        match discovered.presentation {
            CapturePresentation::Indexed { factory, .. } => {
                let control = SourcePreparationControl::new();
                let submission = if let Some(request) = factory.preparation_request() {
                    self.executor.submit_request(request, control.clone())
                } else {
                    let work_executor = Arc::clone(&self.work_executor);
                    let artifact_repository = Arc::clone(&self.artifact_repository);
                    let work = Box::new(move |control: SourcePreparationControl| {
                        let metadata = factory.metadata().map_err(|error| error.to_string())?;
                        if !control.report_metadata(metadata) {
                            return Err("source preparation cancelled".to_owned());
                        }
                        factory
                            .open(artifact_repository, work_executor, &mut |progress| {
                                control.report_progress(progress)
                            })
                            .map(PreparedCaptureData::Indexed)
                            .map_err(|error| error.to_string())
                    });
                    self.executor.submit(work, control.clone())
                };
                match submission {
                    Ok(task) => {
                        self.task = Some(task);
                        self.control = Some(control);
                        self.preparing_update(identity, visible_channels)
                    }
                    Err(error) => {
                        let error = format!("could not start capture preparation worker: {error}");
                        self.status = SourcePreparationStatus::Failed(error.clone());
                        SourcePreparationUpdate::Failed(error)
                    }
                }
            }
            CapturePresentation::InMemory {
                signals,
                duration_us,
            } => {
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity,
                    visible_channels,
                    data: PreparedCaptureData::InMemory {
                        signals,
                        duration_us,
                    },
                })
            }
            CapturePresentation::Channels(channels) => {
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity,
                    visible_channels,
                    data: PreparedCaptureData::Channels(channels),
                })
            }
        }
    }

    fn preparing_update(
        &self,
        identity: String,
        visible_channels: Vec<usize>,
    ) -> SourcePreparationUpdate {
        SourcePreparationUpdate::Preparing(PreparingCapture {
            identity,
            visible_channels,
            metadata: self
                .control
                .as_ref()
                .and_then(SourcePreparationControl::metadata),
            progress: self
                .control
                .as_ref()
                .and_then(SourcePreparationControl::progress),
        })
    }

    fn cancel_active(&mut self) {
        if let Some(control) = self.control.take() {
            control.cancel();
        }
        self.task = None;
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }
}

#[cfg(test)]
mod source_preparation_tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use signal_capture::{
        CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory,
        CaptureIndexPreparationRequest, CaptureMetadata, CaptureSampledWindow,
    };
    use signal_runtime::WorkerOperation;

    use super::super::source_preparation_executor::{
        SourcePreparationResult, SourcePreparationWork,
    };
    use super::*;

    #[derive(Clone, Default)]
    struct ControlledExecutor {
        submissions: Arc<Mutex<VecDeque<ControlledSubmission>>>,
    }

    impl ControlledExecutor {
        fn pending_count(&self) -> usize {
            self.submissions.lock().unwrap().len()
        }

        fn complete_next(&self) {
            let mut submission = self
                .submissions
                .lock()
                .unwrap()
                .pop_front()
                .expect("a preparation task should be pending");
            let result = submission
                .work
                .take()
                .expect("the preparation task should run once")(
                submission.control
            );
            *submission.state.lock().unwrap() = ControlledTaskState::Complete(Some(result));
        }

        fn fail_next(&self, error: &str) {
            let submission = self
                .submissions
                .lock()
                .unwrap()
                .pop_front()
                .expect("a preparation task should be pending");
            *submission.state.lock().unwrap() =
                ControlledTaskState::Complete(Some(Err(error.to_owned())));
        }

        fn disconnect_next(&self) {
            let submission = self
                .submissions
                .lock()
                .unwrap()
                .pop_front()
                .expect("a preparation task should be pending");
            *submission.state.lock().unwrap() = ControlledTaskState::Disconnected;
        }

        fn cancelled_count(&self) -> usize {
            self.submissions
                .lock()
                .unwrap()
                .iter()
                .filter(|submission| {
                    matches!(
                        *submission.state.lock().unwrap(),
                        ControlledTaskState::Cancelled
                    )
                })
                .count()
        }

        fn cancelled_control_count(&self) -> usize {
            self.submissions
                .lock()
                .unwrap()
                .iter()
                .filter(|submission| submission.control.is_cancelled())
                .count()
        }
    }

    impl SourcePreparationExecutor for ControlledExecutor {
        fn submit(
            &self,
            work: SourcePreparationWork,
            control: SourcePreparationControl,
        ) -> Result<Box<dyn SourcePreparationTask>, String> {
            let state = Arc::new(Mutex::new(ControlledTaskState::Pending));
            self.submissions
                .lock()
                .unwrap()
                .push_back(ControlledSubmission {
                    work: Some(work),
                    control,
                    state: state.clone(),
                });
            Ok(Box::new(ControlledTask { state }))
        }
    }

    struct ControlledSubmission {
        work: Option<SourcePreparationWork>,
        control: SourcePreparationControl,
        state: Arc<Mutex<ControlledTaskState>>,
    }

    enum ControlledTaskState {
        Pending,
        Complete(Option<SourcePreparationResult>),
        Disconnected,
        Cancelled,
    }

    struct ControlledTask {
        state: Arc<Mutex<ControlledTaskState>>,
    }

    impl SourcePreparationTask for ControlledTask {
        fn poll(&mut self) -> SourcePreparationTaskUpdate {
            let mut state = self.state.lock().unwrap();
            match &mut *state {
                ControlledTaskState::Pending => SourcePreparationTaskUpdate::Pending,
                ControlledTaskState::Complete(result) => SourcePreparationTaskUpdate::Complete(
                    result
                        .take()
                        .expect("a completed task should be consumed once"),
                ),
                ControlledTaskState::Disconnected => SourcePreparationTaskUpdate::Disconnected,
                ControlledTaskState::Cancelled => SourcePreparationTaskUpdate::Disconnected,
            }
        }
    }

    impl Drop for ControlledTask {
        fn drop(&mut self) {
            let mut state = self.state.lock().unwrap();
            if matches!(*state, ControlledTaskState::Pending) {
                *state = ControlledTaskState::Cancelled;
            }
        }
    }

    struct ImmediateExecutor;

    impl SourcePreparationExecutor for ImmediateExecutor {
        fn submit(
            &self,
            work: SourcePreparationWork,
            control: SourcePreparationControl,
        ) -> Result<Box<dyn SourcePreparationTask>, String> {
            Ok(Box::new(ImmediateTask(Some(work(control)))))
        }
    }

    struct ImmediateTask(Option<SourcePreparationResult>);

    impl SourcePreparationTask for ImmediateTask {
        fn poll(&mut self) -> SourcePreparationTaskUpdate {
            SourcePreparationTaskUpdate::Complete(
                self.0
                    .take()
                    .expect("an immediate preparation task should be consumed once"),
            )
        }
    }

    #[derive(Clone, Default)]
    struct HostedExecutor {
        requests: Arc<Mutex<Vec<CaptureIndexPreparationRequest>>>,
    }

    impl SourcePreparationExecutor for HostedExecutor {
        fn submit(
            &self,
            _work: SourcePreparationWork,
            _control: SourcePreparationControl,
        ) -> Result<Box<dyn SourcePreparationTask>, String> {
            Err("hosted preparation must not use the local work path".to_owned())
        }

        fn submit_request(
            &self,
            request: CaptureIndexPreparationRequest,
            control: SourcePreparationControl,
        ) -> Result<Box<dyn SourcePreparationTask>, String> {
            self.requests.lock().unwrap().push(request);
            assert!(control.report_metadata(test_metadata()));
            Ok(Box::new(ImmediateTask(Some(Ok(
                PreparedCaptureData::Indexed(Box::new(TestIndex {
                    metadata: test_metadata(),
                    identity: signal_artifacts::SourceIdentity::from_bytes([6; 32]),
                })),
            )))))
        }
    }

    struct TestIndex {
        metadata: CaptureMetadata,
        identity: signal_artifacts::SourceIdentity,
    }

    impl CaptureIndex for TestIndex {
        fn display_name(&self) -> String {
            "prepared test".into()
        }

        fn index_identity(&self) -> signal_artifacts::SourceIdentity {
            self.identity
        }

        fn header(&self) -> &CaptureMetadata {
            &self.metadata
        }

        fn capture_duration_us(&self) -> f64 {
            self.metadata.duration_us()
        }

        fn sampled_window(
            &mut self,
            _channels: &[usize],
            start_sample: u64,
            end_sample: u64,
            _target_points: usize,
        ) -> signal_capture::Result<CaptureSampledWindow> {
            Ok(CaptureSampledWindow {
                start_sample,
                end_sample,
                sample_step: 1,
                channels: Vec::new(),
            })
        }
    }

    struct TestFactory {
        open_count: Arc<Mutex<usize>>,
        observed_parallelism: Option<Arc<Mutex<Option<usize>>>>,
    }

    fn test_metadata() -> CaptureMetadata {
        CaptureMetadata {
            total_probes: 1,
            samplerate: "1 MHz".into(),
            samplerate_hz: 1_000_000.0,
            sample_period: 0.000_001,
            total_samples: 10,
            total_blocks: 1,
            samples_per_block: 64,
            probe_names: vec!["D0".into()],
            trigger_sample: None,
        }
    }

    impl CaptureIndexFactory for TestFactory {
        fn display_name(&self) -> String {
            "test factory".into()
        }

        fn metadata(&self) -> signal_capture::Result<CaptureMetadata> {
            Ok(test_metadata())
        }

        fn open(
            self: Box<Self>,
            _artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
            work_executor: Arc<dyn signal_runtime::WorkExecutor>,
            _progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
        ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
            *self.open_count.lock().unwrap() += 1;
            if let Some(observed_parallelism) = &self.observed_parallelism {
                *observed_parallelism.lock().unwrap() = Some(work_executor.available_parallelism());
            }
            Ok(Box::new(TestIndex {
                metadata: test_metadata(),
                identity: signal_artifacts::SourceIdentity::from_bytes([7; 32]),
            }))
        }
    }

    struct FailingFactory;

    impl CaptureIndexFactory for FailingFactory {
        fn display_name(&self) -> String {
            "failing test factory".into()
        }

        fn metadata(&self) -> signal_capture::Result<CaptureMetadata> {
            Ok(test_metadata())
        }

        fn open(
            self: Box<Self>,
            _artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
            _work_executor: Arc<dyn signal_runtime::WorkExecutor>,
            _progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
        ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
            Err(signal_capture::Error::ParseError(
                "controlled index error".into(),
            ))
        }
    }

    struct ProgressFactory;

    impl CaptureIndexFactory for ProgressFactory {
        fn display_name(&self) -> String {
            "progress test factory".into()
        }

        fn metadata(&self) -> signal_capture::Result<CaptureMetadata> {
            Ok(test_metadata())
        }

        fn open(
            self: Box<Self>,
            _artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
            _work_executor: Arc<dyn signal_runtime::WorkExecutor>,
            progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
        ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
            assert!(progress(CaptureIndexBuildProgress {
                completed: 2,
                total: 5,
            }));
            Ok(Box::new(TestIndex {
                metadata: test_metadata(),
                identity: signal_artifacts::SourceIdentity::from_bytes([8; 32]),
            }))
        }
    }

    struct HostedFactory;

    impl CaptureIndexFactory for HostedFactory {
        fn display_name(&self) -> String {
            "hosted test factory".to_owned()
        }

        fn preparation_request(&self) -> Option<CaptureIndexPreparationRequest> {
            Some(CaptureIndexPreparationRequest::new(
                WorkerOperation::new("test.capture-index.prepare/v1").unwrap(),
                vec![1, 2, 3],
            ))
        }

        fn metadata(&self) -> signal_capture::Result<CaptureMetadata> {
            panic!("hosted preparation must not inspect metadata on the caller")
        }

        fn open(
            self: Box<Self>,
            _artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
            _work_executor: Arc<dyn signal_runtime::WorkExecutor>,
            _progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
        ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
            panic!("hosted preparation must not open the index on the caller")
        }
    }

    fn in_memory(identity: &str) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![1, 3],
            presentation: CapturePresentation::InMemory {
                signals: Vec::new(),
                duration_us: 42.0,
            },
        }
    }

    fn indexed(identity: &str, open_count: Arc<Mutex<usize>>) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: signal_artifacts::SourceIdentity::from_bytes([1; 32]),
                factory: Box::new(TestFactory {
                    open_count,
                    observed_parallelism: None,
                }),
            },
        }
    }

    struct FixedWorkExecutor {
        parallelism: usize,
    }

    impl signal_runtime::WorkExecutor for FixedWorkExecutor {
        fn available_parallelism(&self) -> usize {
            self.parallelism
        }

        fn submit(
            &self,
            task: signal_runtime::WorkExecutorTask,
        ) -> std::result::Result<Box<dyn signal_runtime::WorkTask>, String> {
            task();
            Ok(Box::new(signal_runtime::CompletedWorkTask))
        }
    }

    fn failing_indexed(identity: &str) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: signal_artifacts::SourceIdentity::from_bytes([2; 32]),
                factory: Box::new(FailingFactory),
            },
        }
    }

    fn progress_indexed(identity: &str) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: signal_artifacts::SourceIdentity::from_bytes([4; 32]),
                factory: Box::new(ProgressFactory),
            },
        }
    }

    fn hosted_indexed(identity: &str) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: signal_artifacts::SourceIdentity::from_bytes([5; 32]),
                factory: Box::new(HostedFactory),
            },
        }
    }

    #[test]
    fn immediate_capture_is_published_once_and_can_be_reset() {
        let mut preparation = SourcePreparation::new();
        let SourcePreparationUpdate::Ready(prepared) =
            preparation.synchronize(Some(in_memory("capture-a")))
        else {
            panic!("in-memory capture should be ready immediately");
        };
        assert_eq!(prepared.identity, "capture-a");
        assert_eq!(prepared.visible_channels, vec![1, 3]);
        assert_eq!(preparation.status(), SourcePreparationStatus::Ready);
        assert!(matches!(
            preparation.synchronize(Some(in_memory("capture-a"))),
            SourcePreparationUpdate::Unchanged
        ));

        preparation.reset();
        assert_eq!(preparation.status(), SourcePreparationStatus::Empty);
        assert!(matches!(
            preparation.synchronize(Some(in_memory("capture-a"))),
            SourcePreparationUpdate::Ready(_)
        ));
    }

    #[test]
    fn indexed_capture_can_be_held_and_completed_without_a_worker() {
        let executor = ControlledExecutor::default();
        let open_count = Arc::new(Mutex::new(0));
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing(_)
        ));
        assert_eq!(executor.pending_count(), 1);
        assert_eq!(*open_count.lock().unwrap(), 0);
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing(_)
        ));

        executor.complete_next();
        let SourcePreparationUpdate::Ready(prepared) =
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone())))
        else {
            panic!("completed preparation should be published");
        };
        assert!(matches!(prepared.data, PreparedCaptureData::Indexed(_)));
        assert_eq!(*open_count.lock().unwrap(), 1);
        assert_eq!(preparation.status(), SourcePreparationStatus::Ready);
    }

    #[test]
    fn hosted_capture_preparation_never_opens_the_factory_on_the_caller() {
        let executor = HostedExecutor::default();
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));

        let SourcePreparationUpdate::Preparing(preparing) =
            preparation.synchronize(Some(hosted_indexed("hosted-capture")))
        else {
            panic!("hosted capture should publish preparation state");
        };
        assert_eq!(preparing.metadata.unwrap().total_probes, 1);
        let requests = executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].operation().as_str(),
            "test.capture-index.prepare/v1"
        );
        assert_eq!(requests[0].payload(), [1, 2, 3]);
        drop(requests);

        let SourcePreparationUpdate::Ready(prepared) =
            preparation.synchronize(Some(hosted_indexed("hosted-capture")))
        else {
            panic!("hosted preparation result should become ready");
        };
        assert!(matches!(prepared.data, PreparedCaptureData::Indexed(_)));
    }

    #[test]
    fn preparation_reports_progress_for_the_active_generation() {
        let executor = ControlledExecutor::default();
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));

        assert!(matches!(
            preparation.synchronize(Some(progress_indexed("indexed-capture"))),
            SourcePreparationUpdate::Preparing(_)
        ));
        assert_eq!(preparation.snapshot().generation, 1);
        assert_eq!(preparation.snapshot().progress, None);

        executor.complete_next();
        assert_eq!(
            preparation.snapshot().progress,
            Some(CaptureIndexBuildProgress {
                completed: 2,
                total: 5,
            })
        );
        assert!(matches!(
            preparation.synchronize(Some(progress_indexed("indexed-capture"))),
            SourcePreparationUpdate::Ready(_)
        ));
        assert_eq!(preparation.snapshot().progress, None);
    }

    #[test]
    fn replacement_cancels_the_old_generation_before_starting_the_next() {
        let executor = ControlledExecutor::default();
        let first_open_count = Arc::new(Mutex::new(0));
        let second_open_count = Arc::new(Mutex::new(0));
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));

        preparation.synchronize(Some(indexed("first", first_open_count)));
        let first_generation = preparation.snapshot().generation;
        preparation.synchronize(Some(indexed("second", second_open_count)));

        assert_eq!(executor.cancelled_count(), 1);
        assert_eq!(executor.cancelled_control_count(), 1);
        assert!(preparation.snapshot().generation > first_generation);
        assert_eq!(
            preparation.snapshot().status,
            SourcePreparationStatus::Preparing
        );
    }

    #[test]
    fn indexed_capture_receives_the_host_work_executor() {
        let open_count = Arc::new(Mutex::new(0));
        let observed_parallelism = Arc::new(Mutex::new(None));
        let presentation = DiscoveredCapturePresentation {
            identity: "indexed-capture".into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: signal_artifacts::SourceIdentity::from_bytes([3; 32]),
                factory: Box::new(TestFactory {
                    open_count: Arc::clone(&open_count),
                    observed_parallelism: Some(Arc::clone(&observed_parallelism)),
                }),
            },
        };
        let mut preparation = SourcePreparation::with_execution(
            Box::new(ImmediateExecutor),
            Arc::new(FixedWorkExecutor { parallelism: 7 }),
        );

        let SourcePreparationUpdate::Preparing(preparing) =
            preparation.synchronize(Some(presentation))
        else {
            panic!("indexed capture should first publish preparation metadata");
        };
        assert_eq!(preparing.identity, "indexed-capture");
        assert_eq!(preparing.visible_channels, vec![0]);
        assert_eq!(preparing.metadata.unwrap().total_probes, 1);
        assert_eq!(*open_count.lock().unwrap(), 1);
        assert_eq!(*observed_parallelism.lock().unwrap(), Some(7));
    }

    #[test]
    fn indexed_capture_failure_is_reported_without_opening_the_capture() {
        let executor = ControlledExecutor::default();
        let open_count = Arc::new(Mutex::new(0));
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing(_)
        ));

        executor.fail_next("controlled preparation failure");
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Failed(error) if error == "controlled preparation failure"
        ));
        assert_eq!(*open_count.lock().unwrap(), 0);
        assert_eq!(
            preparation.status(),
            SourcePreparationStatus::Failed("controlled preparation failure".into())
        );
    }

    #[test]
    fn indexed_capture_opener_error_is_reported_by_the_immediate_executor() {
        let mut preparation = SourcePreparation::with_executor(Box::new(ImmediateExecutor));
        assert!(matches!(
            preparation.synchronize(Some(failing_indexed("indexed-capture"))),
            SourcePreparationUpdate::Preparing(_)
        ));
        assert!(matches!(
            preparation.synchronize(Some(failing_indexed("indexed-capture"))),
            SourcePreparationUpdate::Failed(error)
                if error == "Parse error: controlled index error"
        ));
    }

    #[test]
    fn reset_discards_a_pending_preparation_task() {
        let executor = ControlledExecutor::default();
        let open_count = Arc::new(Mutex::new(0));
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing(_)
        ));

        preparation.reset();
        assert_eq!(executor.cancelled_count(), 1);
        assert_eq!(*open_count.lock().unwrap(), 0);
        assert_eq!(preparation.status(), SourcePreparationStatus::Empty);
    }

    #[test]
    fn unchanged_discovery_failure_is_reported_once() {
        let mut preparation = SourcePreparation::new();

        assert!(matches!(
            preparation.fail("two sources are enabled".into()),
            SourcePreparationUpdate::Failed(error) if error == "two sources are enabled"
        ));
        assert!(matches!(
            preparation.fail("two sources are enabled".into()),
            SourcePreparationUpdate::Unchanged
        ));
        assert_eq!(
            preparation.status(),
            SourcePreparationStatus::Failed("two sources are enabled".into())
        );
    }

    #[test]
    fn disconnected_preparation_task_is_reported_deterministically() {
        let executor = ControlledExecutor::default();
        let open_count = Arc::new(Mutex::new(0));
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing(_)
        ));

        executor.disconnect_next();
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count))),
            SourcePreparationUpdate::Failed(error)
                if error == "capture preparation worker stopped"
        ));
    }
}
