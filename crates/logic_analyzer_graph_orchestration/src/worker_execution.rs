use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use logic_analyzer_graph_capabilities::node::RuntimeBuilderOverride;
use logic_analyzer_graph_compiler::GraphLowerer;
use logic_analyzer_graph_plan::OutputSubscriptionPlan;
use logic_analyzer_graph_runtime::{
    GraphRunContext, GraphRuntime, InlineSourcePreparationExecutor, LiveRun,
};
use node_graph::api::{GraphState, NodeId};
use signal_artifacts::{
    ArtifactReplicationEvent, ArtifactRepository, MemoryArtifactRepository,
    ReplicatingArtifactRepository,
};
use signal_runtime::{CooperativeAppManagerFactory, InlineWorkExecutor};

const GRAPH_PUMP_BUDGET: usize = 256;
const GRAPH_PUMP_DURATION: Duration = Duration::from_millis(4);
const MAX_REPLICATION_EVENTS: usize = 8;
const MAX_REPLICATION_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Owned command envelope for one worker-hosted processing graph.
#[derive(Clone)]
pub enum GraphWorkerRequest {
    /// Starts a new graph execution generation.
    Start {
        /// Host-assigned generation used to correlate later messages.
        sequence: u64,
        /// Editor graph to lower and execute.
        graph: GraphState,
        /// Retained and visible output selection for the run.
        subscriptions: OutputSubscriptionPlan,
        /// Host cursor markers expressed as `(number, timestamp_ns)`.
        timeline_markers: Vec<(u32, u64)>,
    },
    /// Cancels the active execution generation when it matches the sequence.
    Cancel {
        /// Generation to cancel.
        sequence: u64,
    },
}

/// Bounded result envelope produced by worker-hosted graph execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphWorkerMessage {
    /// The requested generation successfully started.
    Started {
        /// Generation that started.
        sequence: u64,
    },
    /// Progress counters changed for one or more graph nodes.
    Progress {
        /// Generation producing the progress update.
        sequence: u64,
        /// `(node_id, progress_count)` entries for active nodes.
        nodes: Vec<(NodeId, u64)>,
    },
    /// New derived-data artifact replication events are available.
    Artifacts {
        /// Generation that produced the artifacts.
        sequence: u64,
        /// Bounded batch of artifact replication events.
        events: Vec<ArtifactReplicationEvent>,
    },
    /// The generation finished without node failures.
    Finished {
        /// Generation that finished.
        sequence: u64,
    },
    /// The generation could not start or failed during execution.
    Failed {
        /// Generation that failed.
        sequence: u64,
        /// User-presentable failure description.
        message: String,
    },
    /// The generation stopped after a matching cancellation request.
    Cancelled {
        /// Generation that was cancelled.
        sequence: u64,
    },
}

struct WorkerRun {
    sequence: u64,
    run: LiveRun,
    last_progress: Vec<(NodeId, u64)>,
}

/// Platform-neutral state machine for running one complete processing graph in a host worker.
pub struct GraphWorkerRuntime {
    lowerer: GraphLowerer,
    runtime: GraphRuntime,
    repository: Arc<ReplicatingArtifactRepository>,
    active: Option<WorkerRun>,
}

impl GraphWorkerRuntime {
    /// Creates a worker runtime backed by in-memory artifact storage.
    ///
    /// # Parameters
    /// - `builder_overrides`: Host-selected replacements for inventory builders.
    pub fn new(builder_overrides: Vec<RuntimeBuilderOverride>) -> Self {
        Self::with_repository(builder_overrides, Arc::new(MemoryArtifactRepository::new()))
    }

    /// Creates a worker runtime using the supplied artifact repository.
    ///
    /// # Parameters
    /// - `builder_overrides`: Host-selected replacements for inventory builders.
    /// - `artifact_repository`: Durable repository that receives replicated artifacts.
    pub fn with_repository(
        builder_overrides: Vec<RuntimeBuilderOverride>,
        artifact_repository: Arc<dyn ArtifactRepository>,
    ) -> Self {
        let repository = Arc::new(ReplicatingArtifactRepository::new(artifact_repository));
        let mut runtime = GraphRuntime::with_execution(
            Box::new(InlineSourcePreparationExecutor),
            Arc::new(CooperativeAppManagerFactory),
            Arc::new(InlineWorkExecutor),
        );
        runtime.set_artifact_repository(repository.clone());
        Self {
            lowerer: GraphLowerer::with_builder_overrides(builder_overrides),
            runtime,
            repository,
            active: None,
        }
    }

    /// Processes a start or cancellation request and emits immediate messages.
    ///
    /// # Parameters
    /// - `request`: Command to execute.
    /// - `emit`: Callback receiving worker messages in request order.
    pub fn execute_streaming(
        &mut self,
        request: GraphWorkerRequest,
        emit: &mut dyn FnMut(GraphWorkerMessage),
    ) {
        match request {
            GraphWorkerRequest::Start {
                sequence,
                graph,
                subscriptions,
                timeline_markers,
            } => self.start(sequence, graph, subscriptions, timeline_markers, emit),
            GraphWorkerRequest::Cancel { sequence } => self.cancel(sequence, emit),
        }
    }

    /// Advances one bounded interactive processing slice and artifact batch.
    pub fn advance_streaming(&mut self, emit: &mut dyn FnMut(GraphWorkerMessage)) -> bool {
        let (sequence, finished) = {
            let Some(active) = self.active.as_mut() else {
                return false;
            };
            active.run.pump_for(GRAPH_PUMP_BUDGET, GRAPH_PUMP_DURATION);
            let progress = active.run.progress();
            if progress != active.last_progress {
                active.last_progress.clone_from(&progress);
                emit(GraphWorkerMessage::Progress {
                    sequence: active.sequence,
                    nodes: progress,
                });
            }
            (active.sequence, active.run.is_finished())
        };
        if let Err(error) = self.emit_artifacts(sequence, emit) {
            self.active = None;
            emit(GraphWorkerMessage::Failed {
                sequence,
                message: error,
            });
            return false;
        }
        if !finished {
            return true;
        }
        if self.repository.has_pending() {
            return true;
        }
        let failures = self
            .active
            .as_mut()
            .map(|active| active.run.take_node_failures())
            .unwrap_or_default();
        if !failures.is_empty() {
            self.active = None;
            emit(GraphWorkerMessage::Failed {
                sequence,
                message: failures
                    .into_iter()
                    .map(|(_, failure)| format!("{}: {}", failure.node, failure.message))
                    .collect::<Vec<_>>()
                    .join("\n"),
            });
            return false;
        }
        self.active = None;
        emit(GraphWorkerMessage::Finished { sequence });
        false
    }

    /// Returns whether a graph execution generation is currently active.
    pub fn has_active_run(&self) -> bool {
        self.active.is_some()
    }

    fn start(
        &mut self,
        sequence: u64,
        graph: GraphState,
        subscriptions: OutputSubscriptionPlan,
        timeline_markers: Vec<(u32, u64)>,
        emit: &mut dyn FnMut(GraphWorkerMessage),
    ) {
        if self.active.is_some() {
            emit(GraphWorkerMessage::Failed {
                sequence,
                message: "the graph worker already has an active run".to_owned(),
            });
            return;
        }
        if let Err(error) = self.repository.discard_pending() {
            emit(GraphWorkerMessage::Failed {
                sequence,
                message: error.to_string(),
            });
            return;
        }
        if let Err(message) = self.runtime.clear_derived_caches() {
            emit(GraphWorkerMessage::Failed { sequence, message });
            return;
        }
        self.lowerer.set_output_subscriptions(subscriptions);
        let mut context = GraphRunContext::default();
        for (number, timestamp_ns) in timeline_markers {
            context.set_timeline_marker(
                logic_analyzer_graph_capabilities::node_support::TimelineMarkerReference::Cursor {
                    number,
                },
                signal_derived::TimelineMarker::new(timestamp_ns),
            );
        }
        let compiled = match self.lowerer.lower(&graph) {
            Ok(compiled) => compiled,
            Err(errors) => {
                emit(GraphWorkerMessage::Failed {
                    sequence,
                    message: errors
                        .into_iter()
                        .map(|error| error.message)
                        .collect::<Vec<_>>()
                        .join("\n"),
                });
                return;
            }
        };
        match self
            .runtime
            .start(compiled, &mut context, Default::default())
        {
            Ok(run) => {
                self.active = Some(WorkerRun {
                    sequence,
                    run,
                    last_progress: Vec::new(),
                });
                emit(GraphWorkerMessage::Started { sequence });
            }
            Err(errors) => emit(GraphWorkerMessage::Failed {
                sequence,
                message: errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
        }
    }

    fn cancel(&mut self, sequence: u64, emit: &mut dyn FnMut(GraphWorkerMessage)) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.sequence == sequence)
        {
            self.active = None;
            let _ = self.repository.discard_pending();
            emit(GraphWorkerMessage::Cancelled { sequence });
        }
    }

    fn emit_artifacts(
        &self,
        sequence: u64,
        emit: &mut dyn FnMut(GraphWorkerMessage),
    ) -> Result<(), String> {
        let events = self
            .repository
            .drain(MAX_REPLICATION_EVENTS, MAX_REPLICATION_PAYLOAD_BYTES)
            .map_err(|error| error.to_string())?;
        if !events.is_empty() {
            emit(GraphWorkerMessage::Artifacts { sequence, events });
        }
        Ok(())
    }
}

#[cfg(test)]
mod worker_execution_tests {
    use super::*;

    #[test]
    fn compile_failures_are_terminal_without_creating_a_worker_run() {
        let mut runtime = GraphWorkerRuntime::new(Vec::new());
        let mut messages = Vec::new();
        runtime.execute_streaming(
            GraphWorkerRequest::Start {
                sequence: 7,
                graph: GraphState::default(),
                subscriptions: OutputSubscriptionPlan::new(),
                timeline_markers: Vec::new(),
            },
            &mut |message| messages.push(message),
        );
        assert!(matches!(
            messages.as_slice(),
            [GraphWorkerMessage::Failed {
                sequence: 7,
                message,
            }] if message.contains("no sink")
        ));
        assert!(!runtime.has_active_run());
    }
}
