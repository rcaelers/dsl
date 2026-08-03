use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use super::host_protocol::{
    CaptureWorkerMessage, CaptureWorkerReplayRequest, CaptureWorkerRequest,
};
use super::preparation::CaptureIndexPreparationRequest;
use super::query::{CaptureIndexQuery, CaptureIndexQueryExecutor, CaptureIndexQueryUpdate};

/// Thread-safe state machine shared by capture-worker host adapters and proxies.
pub struct CaptureWorkerClient {
    max_outstanding: usize,
    state: Mutex<ClientState>,
}

/// Query executor bound to one worker-owned prepared capture session.
pub struct CaptureWorkerIndexQueryExecutor {
    client: Arc<CaptureWorkerClient>,
    session_id: u64,
}

impl CaptureWorkerIndexQueryExecutor {
    /// Binds an index-query executor to one prepared worker session.
    ///
    /// # Parameters
    /// - `client`: Shared worker request and response state machine.
    /// - `session_id`: Prepared capture session leased by this executor.
    pub fn new(client: Arc<CaptureWorkerClient>, session_id: u64) -> Self {
        Self { client, session_id }
    }
}

impl CaptureIndexQueryExecutor for CaptureWorkerIndexQueryExecutor {
    fn submit(&self, query: CaptureIndexQuery) -> Result<u64, String> {
        self.client.submit_query(self.session_id, query)
    }

    fn poll(&self, request_id: u64) -> CaptureIndexQueryUpdate {
        let mut updates = self.client.take_updates(request_id).into_iter();
        match updates.next() {
            None => CaptureIndexQueryUpdate::Pending,
            Some(CaptureWorkerMessage::Window { window, .. }) => {
                CaptureIndexQueryUpdate::Complete(Ok(window))
            }
            Some(CaptureWorkerMessage::Failed { message, .. }) => {
                CaptureIndexQueryUpdate::Complete(Err(message))
            }
            Some(CaptureWorkerMessage::Cancelled { .. }) => {
                CaptureIndexQueryUpdate::Complete(Err("capture query cancelled".to_owned()))
            }
            Some(
                CaptureWorkerMessage::Progress { .. }
                | CaptureWorkerMessage::Metadata { .. }
                | CaptureWorkerMessage::Prepared { .. }
                | CaptureWorkerMessage::Replay { .. },
            ) => CaptureIndexQueryUpdate::Disconnected,
        }
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.client.cancel(request_id)
    }
}

impl Drop for CaptureWorkerIndexQueryExecutor {
    fn drop(&mut self) {
        self.client.release(self.session_id);
    }
}

#[derive(Default)]
struct ClientState {
    next_sequence: u64,
    pending: BTreeMap<u64, RequestKind>,
    cancelled: BTreeSet<u64>,
    outbound: VecDeque<CaptureWorkerRequest>,
    updates: BTreeMap<u64, VecDeque<CaptureWorkerMessage>>,
    disconnected: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestKind {
    Preparation,
    Query,
    Replay,
}

impl CaptureWorkerClient {
    /// Creates a worker client with a bounded number of outstanding requests.
    pub fn new(max_outstanding: usize) -> Result<Self, String> {
        if max_outstanding == 0 {
            return Err("capture-worker queue must accept at least one request".to_owned());
        }
        Ok(Self {
            max_outstanding,
            state: Mutex::new(ClientState::default()),
        })
    }

    /// Queues capture preparation and returns its opaque request sequence.
    pub fn submit_preparation(
        &self,
        request: CaptureIndexPreparationRequest,
    ) -> Result<u64, String> {
        self.submit(RequestKind::Preparation, |sequence| {
            CaptureWorkerRequest::Prepare { sequence, request }
        })
    }

    /// Queues a bounded sampled-window query for a prepared session.
    ///
    /// # Parameters
    /// - `session_id`: Prepared capture session to query.
    /// - `query`: Channels, sample range, and point budget to request.
    pub fn submit_query(&self, session_id: u64, query: CaptureIndexQuery) -> Result<u64, String> {
        self.submit(RequestKind::Query, |sequence| CaptureWorkerRequest::Query {
            sequence,
            session_id,
            query,
        })
    }

    /// Queues a raw-block replay request for a prepared session.
    ///
    /// # Parameters
    ///
    /// - `session_id`: Prepared capture session to replay.
    /// - `request`: Requested raw block range and channel data.
    pub fn submit_replay(
        &self,
        session_id: u64,
        request: CaptureWorkerReplayRequest,
    ) -> Result<u64, String> {
        self.submit(RequestKind::Replay, |sequence| {
            CaptureWorkerRequest::Replay {
                sequence,
                session_id,
                request,
            }
        })
    }

    /// Requests cancellation of a still-pending request.
    ///
    /// # Parameters
    ///
    /// - `sequence`: Opaque request sequence returned by a submit method.
    pub fn cancel(&self, sequence: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.pending.contains_key(&sequence) {
            return false;
        }
        if !state.cancelled.insert(sequence) {
            return true;
        }
        state
            .outbound
            .push_back(CaptureWorkerRequest::Cancel { sequence });
        true
    }

    /// Releases a prepared-session lease held by a query proxy.
    ///
    /// # Parameters
    ///
    /// - `session_id`: Prepared capture session to release.
    pub fn release(&self, session_id: u64) {
        let mut state = self.state.lock().unwrap();
        if state.disconnected.is_none() {
            state
                .outbound
                .push_back(CaptureWorkerRequest::Release { session_id });
        }
    }

    /// Drains queued protocol requests for delivery to the host worker.
    pub fn drain_requests(&self) -> Vec<CaptureWorkerRequest> {
        self.state.lock().unwrap().outbound.drain(..).collect()
    }

    /// Accepts one worker response after validating its request kind and sequence.
    ///
    /// # Parameters
    ///
    /// - `message`: Worker protocol message to route to the waiting request.
    pub fn publish(&self, mut message: CaptureWorkerMessage) -> Result<(), String> {
        let sequence = message_sequence(&message);
        let mut state = self.state.lock().unwrap();
        let Some(kind) = state.pending.get(&sequence).copied() else {
            return Err(format!(
                "capture worker returned sequence {sequence} with no pending request"
            ));
        };
        validate_message(kind, &message)?;
        if state.cancelled.contains(&sequence) {
            match message {
                CaptureWorkerMessage::Progress { .. } | CaptureWorkerMessage::Metadata { .. } => {
                    return Ok(());
                }
                CaptureWorkerMessage::Prepared { session_id, .. } => {
                    state
                        .outbound
                        .push_back(CaptureWorkerRequest::Release { session_id });
                }
                CaptureWorkerMessage::Window { .. }
                | CaptureWorkerMessage::Replay { .. }
                | CaptureWorkerMessage::Failed { .. }
                | CaptureWorkerMessage::Cancelled { .. } => {}
            }
            message = CaptureWorkerMessage::Cancelled { sequence };
        }
        let terminal = message_is_terminal(&message);
        state
            .updates
            .entry(sequence)
            .or_default()
            .push_back(message);
        if terminal {
            state.pending.remove(&sequence);
            state.cancelled.remove(&sequence);
        }
        Ok(())
    }

    /// Takes updates, leaving its default state.
    ///
    /// # Parameters
    /// - `sequence`: Request sequence whose queued updates are drained.
    pub fn take_updates(&self, sequence: u64) -> Vec<CaptureWorkerMessage> {
        self.state
            .lock()
            .unwrap()
            .updates
            .remove(&sequence)
            .map(|updates| updates.into_iter().collect())
            .unwrap_or_default()
    }

    /// Fails every pending request after worker transport disconnection.
    ///
    /// # Parameters
    ///
    /// - `message`: Diagnostic reason reported to all pending callers.
    pub fn fail_all(&self, message: impl Into<String>) {
        let message = message.into();
        let mut state = self.state.lock().unwrap();
        state.disconnected = Some(message.clone());
        let pending = std::mem::take(&mut state.pending);
        state.cancelled.clear();
        state.outbound.clear();
        for sequence in pending.keys().copied() {
            state
                .updates
                .entry(sequence)
                .or_default()
                .push_back(CaptureWorkerMessage::Failed {
                    sequence,
                    message: message.clone(),
                });
        }
    }

    /// Returns the number of requests awaiting a terminal worker response.
    pub fn outstanding(&self) -> usize {
        self.state.lock().unwrap().pending.len()
    }

    fn submit(
        &self,
        kind: RequestKind,
        request: impl FnOnce(u64) -> CaptureWorkerRequest,
    ) -> Result<u64, String> {
        let mut state = self.state.lock().unwrap();
        if let Some(message) = &state.disconnected {
            return Err(message.clone());
        }
        if state.pending.len() >= self.max_outstanding {
            return Err(format!(
                "capture-worker queue is full ({} outstanding request limit)",
                self.max_outstanding
            ));
        }
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "capture-worker request sequence exhausted".to_owned())?;
        let sequence = state.next_sequence;
        state.pending.insert(sequence, kind);
        state.outbound.push_back(request(sequence));
        Ok(sequence)
    }
}

fn message_sequence(message: &CaptureWorkerMessage) -> u64 {
    match message {
        CaptureWorkerMessage::Progress { sequence, .. }
        | CaptureWorkerMessage::Metadata { sequence, .. }
        | CaptureWorkerMessage::Prepared { sequence, .. }
        | CaptureWorkerMessage::Window { sequence, .. }
        | CaptureWorkerMessage::Replay { sequence, .. }
        | CaptureWorkerMessage::Failed { sequence, .. }
        | CaptureWorkerMessage::Cancelled { sequence } => *sequence,
    }
}

fn message_is_terminal(message: &CaptureWorkerMessage) -> bool {
    matches!(
        message,
        CaptureWorkerMessage::Prepared { .. }
            | CaptureWorkerMessage::Window { .. }
            | CaptureWorkerMessage::Replay { .. }
            | CaptureWorkerMessage::Failed { .. }
            | CaptureWorkerMessage::Cancelled { .. }
    )
}

fn validate_message(kind: RequestKind, message: &CaptureWorkerMessage) -> Result<(), String> {
    let valid = match kind {
        RequestKind::Preparation => matches!(
            message,
            CaptureWorkerMessage::Progress { .. }
                | CaptureWorkerMessage::Metadata { .. }
                | CaptureWorkerMessage::Prepared { .. }
                | CaptureWorkerMessage::Failed { .. }
                | CaptureWorkerMessage::Cancelled { .. }
        ),
        RequestKind::Query => matches!(
            message,
            CaptureWorkerMessage::Window { .. }
                | CaptureWorkerMessage::Failed { .. }
                | CaptureWorkerMessage::Cancelled { .. }
        ),
        RequestKind::Replay => matches!(
            message,
            CaptureWorkerMessage::Replay { .. }
                | CaptureWorkerMessage::Failed { .. }
                | CaptureWorkerMessage::Cancelled { .. }
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "capture worker returned {} for a {} request",
            message_kind(message),
            match kind {
                RequestKind::Preparation => "preparation",
                RequestKind::Query => "query",
                RequestKind::Replay => "replay",
            }
        ))
    }
}

fn message_kind(message: &CaptureWorkerMessage) -> &'static str {
    match message {
        CaptureWorkerMessage::Progress { .. } => "progress",
        CaptureWorkerMessage::Metadata { .. } => "metadata",
        CaptureWorkerMessage::Prepared { .. } => "prepared",
        CaptureWorkerMessage::Window { .. } => "window",
        CaptureWorkerMessage::Replay { .. } => "replay",
        CaptureWorkerMessage::Failed { .. } => "failure",
        CaptureWorkerMessage::Cancelled { .. } => "cancellation",
    }
}

#[cfg(test)]
mod worker_client_tests {
    use super::*;
    use crate::{
        CaptureIndexBuildProgress, CaptureMetadata, CaptureSampledWindow, WorkerOperation,
    };

    fn preparation() -> CaptureIndexPreparationRequest {
        CaptureIndexPreparationRequest::new(
            WorkerOperation::new("test.capture.prepare/v1").unwrap(),
            vec![1, 2, 3],
        )
    }

    fn query() -> CaptureIndexQuery {
        CaptureIndexQuery {
            channels: vec![0],
            start_sample: 10,
            end_sample: 20,
            target_points: 100,
        }
    }

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
    fn client_bounds_requests_and_routes_updates_by_sequence() {
        let client = CaptureWorkerClient::new(2).unwrap();
        let preparation_sequence = client.submit_preparation(preparation()).unwrap();
        let query_sequence = client.submit_query(7, query()).unwrap();

        assert!(client.submit_query(7, query()).is_err());
        assert_eq!(client.outstanding(), 2);
        assert_eq!(client.drain_requests().len(), 2);

        client
            .publish(CaptureWorkerMessage::Progress {
                sequence: preparation_sequence,
                progress: CaptureIndexBuildProgress {
                    completed: 1,
                    total: 2,
                },
            })
            .unwrap();
        client
            .publish(CaptureWorkerMessage::Window {
                sequence: query_sequence,
                window: CaptureSampledWindow {
                    start_sample: 10,
                    end_sample: 20,
                    sample_step: 1,
                    channels: Vec::new(),
                },
            })
            .unwrap();

        assert_eq!(client.take_updates(preparation_sequence).len(), 1);
        assert_eq!(client.take_updates(query_sequence).len(), 1);
        assert_eq!(client.outstanding(), 1);
    }

    #[test]
    fn client_rejects_response_kinds_that_do_not_match_the_request() {
        let client = CaptureWorkerClient::new(1).unwrap();
        let sequence = client.submit_query(7, query()).unwrap();

        let error = client
            .publish(CaptureWorkerMessage::Metadata {
                sequence,
                metadata: metadata(),
            })
            .unwrap_err();

        assert!(error.contains("metadata for a query request"));
        assert_eq!(client.outstanding(), 1);
    }

    #[test]
    fn cancellation_and_disconnect_are_explicit_terminal_updates() {
        let client = CaptureWorkerClient::new(2).unwrap();
        let cancelled = client.submit_query(7, query()).unwrap();
        let disconnected = client.submit_preparation(preparation()).unwrap();
        client.drain_requests();

        assert!(client.cancel(cancelled));
        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Cancel { sequence }] if *sequence == cancelled
        ));
        client
            .publish(CaptureWorkerMessage::Cancelled {
                sequence: cancelled,
            })
            .unwrap();
        client.fail_all("worker disconnected");

        assert!(matches!(
            client.take_updates(cancelled).as_slice(),
            [CaptureWorkerMessage::Cancelled { sequence }] if *sequence == cancelled
        ));
        assert!(matches!(
            client.take_updates(disconnected).as_slice(),
            [CaptureWorkerMessage::Failed { sequence, message }]
                if *sequence == disconnected && message == "worker disconnected"
        ));
        assert_eq!(client.outstanding(), 0);
    }

    #[test]
    fn releasing_a_session_does_not_consume_request_capacity() {
        let client = CaptureWorkerClient::new(1).unwrap();
        client.release(42);

        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Release { session_id: 42 }]
        ));
        assert_eq!(client.outstanding(), 0);
    }

    #[test]
    fn session_query_executor_routes_windows_and_releases_its_session() {
        let client = Arc::new(CaptureWorkerClient::new(1).unwrap());
        let executor = CaptureWorkerIndexQueryExecutor::new(Arc::clone(&client), 42);
        let sequence = executor.submit(query()).unwrap();
        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Query {
                sequence: request_sequence,
                session_id: 42,
                ..
            }] if *request_sequence == sequence
        ));

        let window = CaptureSampledWindow {
            start_sample: 10,
            end_sample: 20,
            sample_step: 1,
            channels: Vec::new(),
        };
        client
            .publish(CaptureWorkerMessage::Window {
                sequence,
                window: window.clone(),
            })
            .unwrap();
        assert!(matches!(
            executor.poll(sequence),
            CaptureIndexQueryUpdate::Complete(Ok(actual)) if actual == window
        ));

        drop(executor);
        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Release { session_id: 42 }]
        ));
    }

    #[test]
    fn prepared_session_that_races_with_cancellation_is_released() {
        let client = CaptureWorkerClient::new(1).unwrap();
        let sequence = client.submit_preparation(preparation()).unwrap();
        client.drain_requests();
        assert!(client.cancel(sequence));
        client.drain_requests();

        client
            .publish(CaptureWorkerMessage::Prepared {
                sequence,
                session_id: 42,
                display_name: "cancelled.dsl".to_owned(),
                source_identity: crate::SourceIdentity::from_bytes([2; 32]),
                index_identity: crate::SourceIdentity::from_bytes([3; 32]),
                metadata: metadata(),
            })
            .unwrap();

        assert!(matches!(
            client.take_updates(sequence).as_slice(),
            [CaptureWorkerMessage::Cancelled {
                sequence: cancelled_sequence
            }] if *cancelled_sequence == sequence
        ));
        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Release { session_id: 42 }]
        ));
        assert_eq!(client.outstanding(), 0);
    }

    #[test]
    fn worker_failure_rejects_later_submissions() {
        let client = CaptureWorkerClient::new(2).unwrap();
        let pending = client.submit_query(7, query()).unwrap();

        client.fail_all("capture worker disconnected");

        assert!(matches!(
            client.take_updates(pending).as_slice(),
            [CaptureWorkerMessage::Failed { sequence, message }]
                if *sequence == pending && message == "capture worker disconnected"
        ));
        assert_eq!(
            client.submit_query(7, query()).unwrap_err(),
            "capture worker disconnected"
        );
        assert!(client.drain_requests().is_empty());
    }

    #[test]
    fn replay_requests_are_bounded_and_accept_only_replay_results() {
        let client = CaptureWorkerClient::new(1).unwrap();
        let request = CaptureWorkerReplayRequest {
            channels: vec![0, 1],
            block: 4,
            start_channel: 0,
            max_payload_bytes: 1024,
        };
        let sequence = client.submit_replay(7, request.clone()).unwrap();
        assert!(matches!(
            client.drain_requests().as_slice(),
            [CaptureWorkerRequest::Replay {
                sequence: actual_sequence,
                session_id: 7,
                request: actual_request,
            }] if *actual_sequence == sequence && actual_request == &request
        ));

        assert!(
            client
                .publish(CaptureWorkerMessage::Window {
                    sequence,
                    window: CaptureSampledWindow {
                        start_sample: 0,
                        end_sample: 1,
                        sample_step: 1,
                        channels: Vec::new(),
                    },
                })
                .is_err()
        );
        client
            .publish(CaptureWorkerMessage::Replay {
                sequence,
                block: 4,
                blocks: Vec::new(),
                next_channel: 2,
            })
            .unwrap();
        assert!(matches!(
            client.take_updates(sequence).as_slice(),
            [CaptureWorkerMessage::Replay { block: 4, .. }]
        ));
    }
}
