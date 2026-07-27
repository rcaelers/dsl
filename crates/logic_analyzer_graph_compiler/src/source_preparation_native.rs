use logic_analyzer_graph_api::node_support::CapturePresentation;

use super::source_preparation_executor::{
    SourcePreparationExecutor, SourcePreparationTask, SourcePreparationTaskUpdate,
};
use super::source_preparation_executor_native::NativeSourcePreparationExecutor;
use super::{
    DiscoveredCapturePresentation, PreparedCapture, PreparedCaptureData, SourcePreparationStatus,
    SourcePreparationUpdate,
};

pub(crate) struct SourcePreparation {
    identity: Option<String>,
    executor: Box<dyn SourcePreparationExecutor>,
    task: Option<Box<dyn SourcePreparationTask>>,
    status: SourcePreparationStatus,
}

impl SourcePreparation {
    pub(crate) fn new() -> Self {
        Self::with_executor(Box::new(NativeSourcePreparationExecutor))
    }

    fn with_executor(executor: Box<dyn SourcePreparationExecutor>) -> Self {
        Self {
            identity: None,
            executor,
            task: None,
            status: SourcePreparationStatus::Empty,
        }
    }

    pub(crate) fn synchronize(
        &mut self,
        discovered: Option<DiscoveredCapturePresentation>,
    ) -> SourcePreparationUpdate {
        let Some(discovered) = discovered else {
            let changed = self.identity.take().is_some();
            self.task = None;
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
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data,
                })
            }
            SourcePreparationTaskUpdate::Complete(Err(error)) => {
                self.task = None;
                self.status = SourcePreparationStatus::Failed(error.clone());
                SourcePreparationUpdate::Failed(error)
            }
            SourcePreparationTaskUpdate::Pending => SourcePreparationUpdate::Preparing,
            SourcePreparationTaskUpdate::Disconnected => {
                self.task = None;
                let error = "capture preparation worker stopped".to_owned();
                self.status = SourcePreparationStatus::Failed(error.clone());
                SourcePreparationUpdate::Failed(error)
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.identity = None;
        self.task = None;
        self.status = SourcePreparationStatus::Empty;
    }

    /// Records a discovery failure once, cancelling any preparation superseded by it.
    pub(crate) fn fail(&mut self, error: String) -> SourcePreparationUpdate {
        let unchanged = matches!(
            &self.status,
            SourcePreparationStatus::Failed(previous) if previous == &error
        );
        self.identity = None;
        self.task = None;
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

    fn start(&mut self, discovered: DiscoveredCapturePresentation) -> SourcePreparationUpdate {
        self.task = None;
        self.status = SourcePreparationStatus::Preparing;
        match discovered.presentation {
            CapturePresentation::Indexed { factory, .. } => {
                let work = Box::new(move || {
                    factory
                        .open(&mut |_| {})
                        .map(PreparedCaptureData::Indexed)
                        .map_err(|error| error.to_string())
                });
                match self.executor.submit(work) {
                    Ok(task) => {
                        self.task = Some(task);
                        SourcePreparationUpdate::Preparing
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
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data: PreparedCaptureData::InMemory {
                        signals,
                        duration_us,
                    },
                })
            }
            CapturePresentation::Channels(channels) => {
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data: PreparedCaptureData::Channels(channels),
                })
            }
        }
    }
}

#[cfg(test)]
mod source_preparation_tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use signal_processing::{
        CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureMetadata,
        CaptureSampledWindow,
    };

    use super::*;
    use crate::source_preparation_executor::{SourcePreparationResult, SourcePreparationWork};

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
                .expect("the preparation task should run once")();
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
    }

    impl SourcePreparationExecutor for ControlledExecutor {
        fn submit(
            &self,
            work: SourcePreparationWork,
        ) -> Result<Box<dyn SourcePreparationTask>, String> {
            let state = Arc::new(Mutex::new(ControlledTaskState::Pending));
            self.submissions
                .lock()
                .unwrap()
                .push_back(ControlledSubmission {
                    work: Some(work),
                    state: state.clone(),
                });
            Ok(Box::new(ControlledTask { state }))
        }
    }

    struct ControlledSubmission {
        work: Option<SourcePreparationWork>,
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
        ) -> Result<Box<dyn SourcePreparationTask>, String> {
            Ok(Box::new(ImmediateTask(Some(work()))))
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

    struct TestIndex {
        metadata: CaptureMetadata,
        path: PathBuf,
    }

    impl CaptureIndex for TestIndex {
        fn display_name(&self) -> String {
            "prepared test".into()
        }

        fn index_path(&self) -> &Path {
            &self.path
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
        ) -> signal_processing::Result<CaptureSampledWindow> {
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
    }

    impl CaptureIndexFactory for TestFactory {
        fn display_name(&self) -> String {
            "test factory".into()
        }

        fn open(
            self: Box<Self>,
            _progress: &mut dyn FnMut(CaptureIndexBuildProgress),
        ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
            *self.open_count.lock().unwrap() += 1;
            Ok(Box::new(TestIndex {
                metadata: CaptureMetadata {
                    total_probes: 1,
                    samplerate: "1 MHz".into(),
                    samplerate_hz: 1_000_000.0,
                    sample_period: 0.000_001,
                    total_samples: 10,
                    total_blocks: 1,
                    samples_per_block: 64,
                    probe_names: vec!["D0".into()],
                    trigger_sample: None,
                },
                path: "prepared-test.index".into(),
            }))
        }
    }

    struct FailingFactory;

    impl CaptureIndexFactory for FailingFactory {
        fn display_name(&self) -> String {
            "failing test factory".into()
        }

        fn open(
            self: Box<Self>,
            _progress: &mut dyn FnMut(CaptureIndexBuildProgress),
        ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
            Err(signal_processing::Error::ParseError(
                "controlled index error".into(),
            ))
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
                identity: "capture.dsl".into(),
                factory: Box::new(TestFactory { open_count }),
            },
        }
    }

    fn failing_indexed(identity: &str) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: "failing-capture.dsl".into(),
                factory: Box::new(FailingFactory),
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
            SourcePreparationUpdate::Preparing
        ));
        assert_eq!(executor.pending_count(), 1);
        assert_eq!(*open_count.lock().unwrap(), 0);
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing
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
    fn indexed_capture_failure_is_reported_without_opening_the_capture() {
        let executor = ControlledExecutor::default();
        let open_count = Arc::new(Mutex::new(0));
        let mut preparation = SourcePreparation::with_executor(Box::new(executor.clone()));
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count.clone()))),
            SourcePreparationUpdate::Preparing
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
            SourcePreparationUpdate::Preparing
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
            SourcePreparationUpdate::Preparing
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
            SourcePreparationUpdate::Preparing
        ));

        executor.disconnect_next();
        assert!(matches!(
            preparation.synchronize(Some(indexed("indexed-capture", open_count))),
            SourcePreparationUpdate::Failed(error)
                if error == "capture preparation worker stopped"
        ));
    }
}
