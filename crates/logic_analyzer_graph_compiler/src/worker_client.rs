use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use logic_analyzer_graph_api::node_support::TimelineMarkerReference;
use node_graph::api::GraphState;
use signal_processing::{ArtifactReplicationReceiver, ArtifactRepository};

use super::{CompileCtx, GraphWorkerMessage, GraphWorkerRequest, OutputSubscriptionPlan};

/// Thread-safe queue shared by a host worker adapter and graph-run proxies.
pub struct GraphWorkerClient {
    max_outstanding: usize,
    receiver: Mutex<ArtifactReplicationReceiver>,
    state: Mutex<ClientState>,
}

#[derive(Default)]
struct ClientState {
    next_sequence: u64,
    pending: BTreeSet<u64>,
    cancelled: BTreeSet<u64>,
    outbound: VecDeque<GraphWorkerRequest>,
    updates: BTreeMap<u64, VecDeque<GraphWorkerMessage>>,
    disconnected: Option<String>,
}

impl GraphWorkerClient {
    /// Creates a bounded client for sending graph runs to a host worker.
    ///
    /// # Parameters
    /// - `max_outstanding`: Maximum worker runs awaiting a terminal message.
    /// - `artifact_repository`: Repository into which replicated worker artifacts are applied.
    pub fn new(
        max_outstanding: usize,
        artifact_repository: Arc<dyn ArtifactRepository>,
    ) -> Result<Self, String> {
        if max_outstanding == 0 {
            return Err("graph-worker queue must accept at least one request".to_owned());
        }
        Ok(Self {
            max_outstanding,
            receiver: Mutex::new(ArtifactReplicationReceiver::new(artifact_repository)),
            state: Mutex::new(ClientState::default()),
        })
    }

    /// Enqueues a graph-execution request and returns its correlation sequence.
    ///
    /// # Parameters
    /// - `graph`: Editor graph to execute in the worker.
    /// - `subscriptions`: Retained and visible output selection.
    /// - `context`: Timeline markers and other run-scoped context.
    pub fn start(
        &self,
        graph: GraphState,
        subscriptions: OutputSubscriptionPlan,
        context: &CompileCtx,
    ) -> Result<u64, String> {
        let timeline_markers = context
            .timeline_markers()
            .map(|(reference, marker)| match reference {
                TimelineMarkerReference::Cursor { number } => (*number, marker.timestamp_ns),
            })
            .collect();
        let mut state = self.state.lock().unwrap();
        if let Some(message) = &state.disconnected {
            return Err(message.clone());
        }
        if state.pending.len() >= self.max_outstanding {
            return Err(format!(
                "graph-worker queue is full ({} outstanding request limit)",
                self.max_outstanding
            ));
        }
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "graph-worker request sequence exhausted".to_owned())?;
        let sequence = state.next_sequence;
        state.pending.insert(sequence);
        state.outbound.push_back(GraphWorkerRequest::Start {
            sequence,
            graph,
            subscriptions,
            timeline_markers,
        });
        Ok(sequence)
    }

    /// Requests cancellation of one pending execution sequence.
    pub fn cancel(&self, sequence: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.pending.contains(&sequence) {
            return false;
        }
        if state.cancelled.insert(sequence) {
            state
                .outbound
                .push_back(GraphWorkerRequest::Cancel { sequence });
        }
        true
    }

    /// Drains outbound requests for delivery by the host worker adapter.
    pub fn drain_requests(&self) -> Vec<GraphWorkerRequest> {
        self.state.lock().unwrap().outbound.drain(..).collect()
    }

    /// Applies one worker message and makes it available to the matching caller.
    pub fn publish(&self, message: GraphWorkerMessage) -> Result<(), String> {
        let sequence = message_sequence(&message);
        let mut state = self.state.lock().unwrap();
        if !state.pending.contains(&sequence) {
            return Err(format!(
                "graph worker returned sequence {sequence} with no pending request"
            ));
        }
        if state.cancelled.contains(&sequence)
            && matches!(
                message,
                GraphWorkerMessage::Started { .. }
                    | GraphWorkerMessage::Progress { .. }
                    | GraphWorkerMessage::Artifacts { .. }
            )
        {
            return Ok(());
        }
        if let GraphWorkerMessage::Artifacts { events, .. } = &message {
            let mut receiver = self.receiver.lock().unwrap();
            for event in events.iter().cloned() {
                receiver.apply(event).map_err(|error| error.to_string())?;
            }
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
    /// - `sequence`: Execution sequence whose queued updates should be drained.
    pub fn take_updates(&self, sequence: u64) -> Vec<GraphWorkerMessage> {
        self.state
            .lock()
            .unwrap()
            .updates
            .remove(&sequence)
            .map(|updates| updates.into_iter().collect())
            .unwrap_or_default()
    }

    /// Fails every pending execution after the worker transport disconnects.
    ///
    /// # Parameters
    /// - `message`: User-presentable transport failure description.
    pub fn fail_all(&self, message: impl Into<String>) {
        let message = message.into();
        let mut state = self.state.lock().unwrap();
        state.disconnected = Some(message.clone());
        let pending = std::mem::take(&mut state.pending);
        state.cancelled.clear();
        state.outbound.clear();
        for sequence in pending {
            state
                .updates
                .entry(sequence)
                .or_default()
                .push_back(GraphWorkerMessage::Failed {
                    sequence,
                    message: message.clone(),
                });
        }
    }

    /// Returns the number of requests awaiting terminal worker messages.
    pub fn outstanding(&self) -> usize {
        self.state.lock().unwrap().pending.len()
    }
}

fn message_sequence(message: &GraphWorkerMessage) -> u64 {
    match message {
        GraphWorkerMessage::Started { sequence }
        | GraphWorkerMessage::Progress { sequence, .. }
        | GraphWorkerMessage::Artifacts { sequence, .. }
        | GraphWorkerMessage::Finished { sequence }
        | GraphWorkerMessage::Failed { sequence, .. }
        | GraphWorkerMessage::Cancelled { sequence } => *sequence,
    }
}

fn message_is_terminal(message: &GraphWorkerMessage) -> bool {
    matches!(
        message,
        GraphWorkerMessage::Finished { .. }
            | GraphWorkerMessage::Failed { .. }
            | GraphWorkerMessage::Cancelled { .. }
    )
}

#[cfg(test)]
mod worker_client_tests {
    use signal_processing::{ArtifactKey, ArtifactNamespace, MemoryArtifactRepository};

    use super::*;

    #[test]
    fn client_applies_artifacts_before_reporting_completion() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let client = GraphWorkerClient::new(1, Arc::clone(&repository)).unwrap();
        let sequence = client
            .start(
                GraphState::default(),
                OutputSubscriptionPlan::new(),
                &CompileCtx::default(),
            )
            .unwrap();
        let _ = client.drain_requests();
        let identity = signal_processing::SourceIdentity::from_bytes([5; 32]);
        client
            .publish(GraphWorkerMessage::Artifacts {
                sequence,
                events: vec![
                    signal_processing::ArtifactReplicationEvent::PublishedChunk {
                        namespace: "worker-client-test".to_owned(),
                        identity,
                        offset: 0,
                        total_length: 4,
                        data: b"done".to_vec(),
                        complete: true,
                    },
                ],
            })
            .unwrap();
        client
            .publish(GraphWorkerMessage::Finished { sequence })
            .unwrap();

        let key = ArtifactKey::new(
            ArtifactNamespace::new("worker-client-test").unwrap(),
            identity,
        );
        assert!(repository.open(&key).unwrap().is_some());
        assert!(matches!(
            client.take_updates(sequence).last(),
            Some(GraphWorkerMessage::Finished { .. })
        ));
    }
}
