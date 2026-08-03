use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    CaptureIndex, CaptureMetadata, CaptureSampledWindow, CaptureSampledWindowPoll, Error, Result,
    SourceIdentity,
};

/// One bounded sampled-window request submitted to a host-owned index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureIndexQuery {
    pub channels: Vec<u64>,
    pub start_sample: u64,
    pub end_sample: u64,
    pub target_points: u64,
}

/// Current state of one host-owned sampled-window request.
pub enum CaptureIndexQueryUpdate {
    Pending,
    Complete(std::result::Result<CaptureSampledWindow, String>),
    Disconnected,
}

/// Host transport for bounded capture-index queries.
///
/// The transport is bound to one prepared index. Implementations may execute
/// locally, on a native worker, or through a browser worker. Request IDs are
/// opaque to the consumer and unique for the lifetime of the transport.
pub trait CaptureIndexQueryExecutor: Send + Sync {
    fn submit(&self, query: CaptureIndexQuery) -> std::result::Result<u64, String>;

    fn poll(&self, request_id: u64) -> CaptureIndexQueryUpdate;

    fn cancel(&self, request_id: u64) -> bool;
}

/// A provider-neutral `CaptureIndex` backed by bounded host queries.
pub struct CaptureIndexProxy {
    display_name: String,
    identity: SourceIdentity,
    metadata: CaptureMetadata,
    executor: Arc<dyn CaptureIndexQueryExecutor>,
    active: Option<ActiveQuery>,
}

struct ActiveQuery {
    request_id: u64,
    query: CaptureIndexQuery,
}

impl CaptureIndexProxy {
    pub fn new(
        display_name: impl Into<String>,
        identity: SourceIdentity,
        metadata: CaptureMetadata,
        executor: Arc<dyn CaptureIndexQueryExecutor>,
    ) -> Self {
        Self {
            display_name: display_name.into(),
            identity,
            metadata,
            executor,
            active: None,
        }
    }

    fn poll_query(&mut self, query: CaptureIndexQuery) -> Result<CaptureSampledWindowPoll> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.query != query)
            && let Some(active) = self.active.take()
        {
            self.executor.cancel(active.request_id);
        }

        if self.active.is_none() {
            let request_id = self
                .executor
                .submit(query.clone())
                .map_err(Error::CaptureQuery)?;
            self.active = Some(ActiveQuery { request_id, query });
        }

        let request_id = self
            .active
            .as_ref()
            .expect("an active query was submitted above")
            .request_id;
        match self.executor.poll(request_id) {
            CaptureIndexQueryUpdate::Pending => Ok(CaptureSampledWindowPoll::Pending),
            CaptureIndexQueryUpdate::Complete(Ok(window)) => {
                self.active = None;
                Ok(CaptureSampledWindowPoll::Ready(window))
            }
            CaptureIndexQueryUpdate::Complete(Err(error)) => {
                self.active = None;
                Err(Error::CaptureQuery(error))
            }
            CaptureIndexQueryUpdate::Disconnected => {
                self.active = None;
                Err(Error::CaptureQuery(
                    "capture-index query host disconnected".to_owned(),
                ))
            }
        }
    }
}

impl CaptureIndex for CaptureIndexProxy {
    fn display_name(&self) -> String {
        self.display_name.clone()
    }

    fn index_identity(&self) -> SourceIdentity {
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
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        match self.poll_sampled_window(channels, start_sample, end_sample, target_points)? {
            CaptureSampledWindowPoll::Pending => Err(Error::CaptureQueryPending),
            CaptureSampledWindowPoll::Ready(window) => Ok(window),
        }
    }

    fn poll_sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindowPoll> {
        self.poll_query(CaptureIndexQuery {
            channels: channels.iter().map(|channel| *channel as u64).collect(),
            start_sample,
            end_sample,
            target_points: target_points as u64,
        })
    }
}

impl Drop for CaptureIndexProxy {
    fn drop(&mut self) {
        if let Some(active) = self.active.take() {
            self.executor.cancel(active.request_id);
        }
    }
}

#[cfg(test)]
mod query_tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct TestExecutor {
        state: Mutex<TestState>,
    }

    #[derive(Default)]
    struct TestState {
        next_request: u64,
        submitted: BTreeMap<u64, CaptureIndexQuery>,
        completed: BTreeMap<u64, std::result::Result<CaptureSampledWindow, String>>,
        cancelled: BTreeSet<u64>,
    }

    impl TestExecutor {
        fn complete(&self, request_id: u64, window: CaptureSampledWindow) {
            self.state
                .lock()
                .unwrap()
                .completed
                .insert(request_id, Ok(window));
        }

        fn request_ids(&self) -> Vec<u64> {
            self.state
                .lock()
                .unwrap()
                .submitted
                .keys()
                .copied()
                .collect()
        }

        fn was_cancelled(&self, request_id: u64) -> bool {
            self.state.lock().unwrap().cancelled.contains(&request_id)
        }
    }

    impl CaptureIndexQueryExecutor for TestExecutor {
        fn submit(&self, query: CaptureIndexQuery) -> std::result::Result<u64, String> {
            let mut state = self.state.lock().unwrap();
            state.next_request += 1;
            let request_id = state.next_request;
            state.submitted.insert(request_id, query);
            Ok(request_id)
        }

        fn poll(&self, request_id: u64) -> CaptureIndexQueryUpdate {
            self.state
                .lock()
                .unwrap()
                .completed
                .remove(&request_id)
                .map(CaptureIndexQueryUpdate::Complete)
                .unwrap_or(CaptureIndexQueryUpdate::Pending)
        }

        fn cancel(&self, request_id: u64) -> bool {
            self.state.lock().unwrap().cancelled.insert(request_id)
        }
    }

    fn metadata() -> CaptureMetadata {
        CaptureMetadata {
            total_probes: 2,
            samplerate: "1 MHz".to_owned(),
            samplerate_hz: 1_000_000.0,
            sample_period: 0.000_001,
            total_samples: 100,
            total_blocks: 1,
            samples_per_block: 100,
            probe_names: vec!["D0".to_owned(), "D1".to_owned()],
            trigger_sample: None,
        }
    }

    fn window(start_sample: u64, end_sample: u64) -> CaptureSampledWindow {
        CaptureSampledWindow {
            start_sample,
            end_sample,
            sample_step: 1,
            channels: Vec::new(),
        }
    }

    #[test]
    fn proxy_polls_one_request_until_its_bounded_result_arrives() {
        let executor = Arc::new(TestExecutor::default());
        let mut proxy = CaptureIndexProxy::new(
            "capture",
            SourceIdentity::from_bytes([7; 32]),
            metadata(),
            executor.clone(),
        );

        assert_eq!(
            proxy.poll_sampled_window(&[0], 10, 20, 100).unwrap(),
            CaptureSampledWindowPoll::Pending
        );
        assert_eq!(executor.request_ids(), [1]);

        executor.complete(1, window(10, 20));
        assert_eq!(
            proxy.poll_sampled_window(&[0], 10, 20, 100).unwrap(),
            CaptureSampledWindowPoll::Ready(window(10, 20))
        );
        assert_eq!(executor.request_ids(), [1]);
    }

    #[test]
    fn proxy_cancels_a_stale_viewport_before_submitting_its_replacement() {
        let executor = Arc::new(TestExecutor::default());
        let mut proxy = CaptureIndexProxy::new(
            "capture",
            SourceIdentity::from_bytes([8; 32]),
            metadata(),
            executor.clone(),
        );

        assert_eq!(
            proxy.poll_sampled_window(&[0], 0, 20, 100).unwrap(),
            CaptureSampledWindowPoll::Pending
        );
        assert_eq!(
            proxy.poll_sampled_window(&[1], 20, 40, 100).unwrap(),
            CaptureSampledWindowPoll::Pending
        );

        assert!(executor.was_cancelled(1));
        assert_eq!(executor.request_ids(), [1, 2]);
    }
}
