use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use signal_processing::{
    CaptureIndexBuildProgress, CaptureIndexPreparationRequest, CaptureIndexProxy, CaptureMetadata,
    CaptureWorkerClient, CaptureWorkerIndexQueryExecutor, CaptureWorkerMessage,
};

use super::PreparedCaptureData;

/// Result produced when a source-preparation task completes.
pub type SourcePreparationResult = Result<PreparedCaptureData, String>;

/// One source-preparation operation accepted by a host executor.
pub type SourcePreparationWork =
    Box<dyn FnOnce(SourcePreparationControl) -> SourcePreparationResult + Send + 'static>;

/// Shared progress and cancellation state for one preparation generation.
#[derive(Clone)]
pub struct SourcePreparationControl {
    inner: Arc<SourcePreparationControlState>,
}

struct SourcePreparationControlState {
    cancelled: AtomicBool,
    metadata: Mutex<Option<CaptureMetadata>>,
    progress: Mutex<Option<CaptureIndexBuildProgress>>,
}

impl SourcePreparationControl {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SourcePreparationControlState {
                cancelled: AtomicBool::new(false),
                metadata: Mutex::new(None),
                progress: Mutex::new(None),
            }),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn report_progress(&self, progress: CaptureIndexBuildProgress) -> bool {
        if self.is_cancelled() {
            return false;
        }
        *self.inner.progress.lock().unwrap() = Some(progress);
        true
    }

    pub fn report_metadata(&self, metadata: CaptureMetadata) -> bool {
        if self.is_cancelled() {
            return false;
        }
        *self.inner.metadata.lock().unwrap() = Some(metadata);
        true
    }

    pub(crate) fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn progress(&self) -> Option<CaptureIndexBuildProgress> {
        *self.inner.progress.lock().unwrap()
    }

    pub(crate) fn metadata(&self) -> Option<CaptureMetadata> {
        self.inner.metadata.lock().unwrap().clone()
    }
}

impl Default for SourcePreparationControl {
    fn default() -> Self {
        Self::new()
    }
}

/// The current state of one submitted source-preparation operation.
pub enum SourcePreparationTaskUpdate {
    Pending,
    Complete(SourcePreparationResult),
    Disconnected,
}

/// A host-owned source-preparation operation that the compiler can poll.
pub trait SourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate;
}

/// Host execution contract for finite-source preparation.
///
/// The compiler owns the task data model and polls the returned task. Hosts
/// decide whether work runs inline, on native workers, or in a future browser
/// worker without introducing target selection into compiler behavior.
pub trait SourcePreparationExecutor: Send + Sync {
    fn submit(
        &self,
        work: SourcePreparationWork,
        control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String>;

    /// Submits an opaque preparation request that must execute in a
    /// host-owned context, such as the worker that owns a browser file.
    fn submit_request(
        &self,
        request: CaptureIndexPreparationRequest,
        _control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        Err(format!(
            "capture-index preparation operation '{}' is unavailable",
            request.operation().as_str()
        ))
    }
}

/// Portable executor for hosts that advance preparation inline.
pub struct InlineSourcePreparationExecutor;

impl SourcePreparationExecutor for InlineSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
        control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        Ok(Box::new(InlineSourcePreparationTask {
            result: Some(work(control)),
        }))
    }
}

/// Source-preparation executor backed by a stateful capture-worker client.
///
/// Local factories delegate to `local`; opaque host requests use the worker
/// client and produce a query proxy bound to the prepared worker session.
pub struct CaptureWorkerSourcePreparationExecutor {
    client: Arc<CaptureWorkerClient>,
    local: Box<dyn SourcePreparationExecutor>,
}

impl CaptureWorkerSourcePreparationExecutor {
    pub fn new(
        client: Arc<CaptureWorkerClient>,
        local: Box<dyn SourcePreparationExecutor>,
    ) -> Self {
        Self { client, local }
    }
}

impl SourcePreparationExecutor for CaptureWorkerSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
        control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        self.local.submit(work, control)
    }

    fn submit_request(
        &self,
        request: CaptureIndexPreparationRequest,
        control: SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        let sequence = self.client.submit_preparation(request)?;
        Ok(Box::new(CaptureWorkerSourcePreparationTask {
            client: Arc::clone(&self.client),
            control,
            sequence,
            terminal: false,
        }))
    }
}

struct CaptureWorkerSourcePreparationTask {
    client: Arc<CaptureWorkerClient>,
    control: SourcePreparationControl,
    sequence: u64,
    terminal: bool,
}

impl SourcePreparationTask for CaptureWorkerSourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate {
        if self.terminal {
            return SourcePreparationTaskUpdate::Disconnected;
        }
        for message in self.client.take_updates(self.sequence) {
            match message {
                CaptureWorkerMessage::Progress { progress, .. } => {
                    self.control.report_progress(progress);
                }
                CaptureWorkerMessage::Metadata { metadata, .. } => {
                    self.control.report_metadata(metadata);
                }
                CaptureWorkerMessage::Prepared {
                    session_id,
                    display_name,
                    index_identity,
                    metadata,
                    ..
                } => {
                    self.terminal = true;
                    if !self.control.report_metadata(metadata.clone()) {
                        self.client.release(session_id);
                        return SourcePreparationTaskUpdate::Complete(Err(
                            "source preparation cancelled".to_owned(),
                        ));
                    }
                    let query_executor = Arc::new(CaptureWorkerIndexQueryExecutor::new(
                        Arc::clone(&self.client),
                        session_id,
                    ));
                    let proxy = CaptureIndexProxy::new(
                        display_name,
                        index_identity,
                        metadata,
                        query_executor,
                    );
                    return SourcePreparationTaskUpdate::Complete(Ok(
                        PreparedCaptureData::Indexed(Box::new(proxy)),
                    ));
                }
                CaptureWorkerMessage::Failed { message, .. } => {
                    self.terminal = true;
                    return SourcePreparationTaskUpdate::Complete(Err(message));
                }
                CaptureWorkerMessage::Cancelled { .. } => {
                    self.terminal = true;
                    return SourcePreparationTaskUpdate::Complete(Err(
                        "source preparation cancelled".to_owned(),
                    ));
                }
                CaptureWorkerMessage::Window { .. } | CaptureWorkerMessage::Replay { .. } => {
                    self.terminal = true;
                    return SourcePreparationTaskUpdate::Complete(Err(
                        "capture worker returned data for a preparation request".to_owned(),
                    ));
                }
            }
        }
        SourcePreparationTaskUpdate::Pending
    }
}

impl Drop for CaptureWorkerSourcePreparationTask {
    fn drop(&mut self) {
        if !self.terminal {
            self.client.cancel(self.sequence);
        }
    }
}

struct InlineSourcePreparationTask {
    result: Option<SourcePreparationResult>,
}

impl SourcePreparationTask for InlineSourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate {
        self.result
            .take()
            .map(SourcePreparationTaskUpdate::Complete)
            .unwrap_or(SourcePreparationTaskUpdate::Disconnected)
    }
}

#[cfg(test)]
mod source_preparation_executor_tests {
    use signal_processing::{
        CaptureIndexPreparationRequest, CaptureSampledWindow, CaptureSampledWindowPoll,
        CaptureWorkerRequest, SourceIdentity, WorkerOperation,
    };

    use super::*;

    fn metadata() -> CaptureMetadata {
        CaptureMetadata {
            total_probes: 1,
            samplerate: "1 MHz".to_owned(),
            samplerate_hz: 1_000_000.0,
            sample_period: 0.000_001,
            total_samples: 100,
            total_blocks: 1,
            samples_per_block: 100,
            probe_names: vec!["D0".to_owned()],
            trigger_sample: None,
        }
    }

    #[test]
    fn worker_preparation_builds_a_proxy_bound_to_the_prepared_session() {
        let client = Arc::new(CaptureWorkerClient::new(4).unwrap());
        let executor = CaptureWorkerSourcePreparationExecutor::new(
            Arc::clone(&client),
            Box::new(InlineSourcePreparationExecutor),
        );
        let control = SourcePreparationControl::new();
        let mut task = executor
            .submit_request(
                CaptureIndexPreparationRequest::new(
                    WorkerOperation::new("test.capture.prepare/v1").unwrap(),
                    vec![1, 2, 3],
                ),
                control.clone(),
            )
            .unwrap();
        let requests = client.drain_requests();
        let [CaptureWorkerRequest::Prepare { sequence, .. }] = requests.as_slice() else {
            panic!("preparation should enqueue one worker request");
        };
        let preparation_sequence = *sequence;

        client
            .publish(CaptureWorkerMessage::Metadata {
                sequence: preparation_sequence,
                metadata: metadata(),
            })
            .unwrap();
        client
            .publish(CaptureWorkerMessage::Progress {
                sequence: preparation_sequence,
                progress: CaptureIndexBuildProgress {
                    completed: 1,
                    total: 2,
                },
            })
            .unwrap();
        assert!(matches!(task.poll(), SourcePreparationTaskUpdate::Pending));
        assert_eq!(control.metadata().unwrap(), metadata());
        assert_eq!(
            control.progress(),
            Some(CaptureIndexBuildProgress {
                completed: 1,
                total: 2,
            })
        );

        client
            .publish(CaptureWorkerMessage::Prepared {
                sequence: preparation_sequence,
                session_id: 42,
                display_name: "capture.dsl".to_owned(),
                source_identity: SourceIdentity::from_bytes([8; 32]),
                index_identity: SourceIdentity::from_bytes([9; 32]),
                metadata: metadata(),
            })
            .unwrap();
        let SourcePreparationTaskUpdate::Complete(Ok(PreparedCaptureData::Indexed(mut index))) =
            task.poll()
        else {
            panic!("prepared worker session should produce a proxy index");
        };
        assert_eq!(index.index_identity(), SourceIdentity::from_bytes([9; 32]));

        assert_eq!(
            index.poll_sampled_window(&[0], 10, 20, 100).unwrap(),
            CaptureSampledWindowPoll::Pending
        );
        let requests = client.drain_requests();
        let [
            CaptureWorkerRequest::Query {
                sequence,
                session_id: 42,
                ..
            },
        ] = requests.as_slice()
        else {
            panic!("proxy should query the prepared worker session");
        };
        let query_sequence = *sequence;
        let window = CaptureSampledWindow {
            start_sample: 10,
            end_sample: 20,
            sample_step: 1,
            channels: Vec::new(),
        };
        client
            .publish(CaptureWorkerMessage::Window {
                sequence: query_sequence,
                window: window.clone(),
            })
            .unwrap();
        assert_eq!(
            index.poll_sampled_window(&[0], 10, 20, 100).unwrap(),
            CaptureSampledWindowPoll::Ready(window)
        );

        drop(index);
        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Release { session_id: 42 }]
        ));
    }
}
