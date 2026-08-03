use std::sync::{Arc, RwLock};

use node_graph::api::NodeId;
use signal_processing::DerivedLanes;

use super::{CollectedOutputSubscription, CollectedTableSubscription, SamplingOverlayCandidate};

/// Immutable application-facing snapshot of one processing run's produced data.
#[derive(Clone)]
pub struct RunData {
    derived_lanes: DerivedLanes,
    output_subscriptions: Vec<CollectedOutputSubscription>,
    table_subscriptions: Vec<CollectedTableSubscription>,
    sampling_overlays: Vec<SamplingOverlayCandidate>,
    diagnostics: RunDiagnosticRegistry,
    source_readiness: SourceReadinessRegistry,
}

impl RunData {
    pub(crate) fn new(
        derived_lanes: DerivedLanes,
        output_subscriptions: Vec<CollectedOutputSubscription>,
        table_subscriptions: Vec<CollectedTableSubscription>,
        sampling_overlays: Vec<SamplingOverlayCandidate>,
        diagnostics: RunDiagnosticRegistry,
        source_readiness: SourceReadinessRegistry,
    ) -> Self {
        Self {
            derived_lanes,
            output_subscriptions,
            table_subscriptions,
            sampling_overlays,
            diagnostics,
            source_readiness,
        }
    }

    /// Returns derived lanes produced by the completed or active run.
    pub fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    /// Returns retained-lane subscriptions requested by graph sinks.
    pub fn output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.output_subscriptions
    }

    /// Returns decoder-table subscriptions requested by graph sinks.
    pub fn table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.table_subscriptions
    }

    /// Returns sampling-overlay candidates reconstructed from graph inputs.
    pub fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        &self.sampling_overlays
    }

    /// Returns the run-scoped diagnostic publication registry.
    pub fn diagnostics(&self) -> &RunDiagnosticRegistry {
        &self.diagnostics
    }

    /// Returns the run-scoped source-artifact readiness registry.
    pub fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }
}

/// Severity of a non-fatal diagnostic published while executing a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunDiagnosticSeverity {
    /// Informational state that does not need user action.
    Info,
    /// Condition the user may need to review.
    Warning,
    /// Failure affecting a node or its produced data.
    Error,
}

/// One user-presentable diagnostic emitted by a processing run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDiagnostic {
    /// Node that emitted the diagnostic, if it is node-specific.
    pub node: Option<NodeId>,
    /// User-impact severity.
    pub severity: RunDiagnosticSeverity,
    /// User-presentable explanation.
    pub message: String,
}

/// Thread-safe publication point for diagnostics produced by one run.
#[derive(Clone, Debug, Default)]
pub struct RunDiagnosticRegistry {
    inner: Arc<RwLock<Vec<RunDiagnostic>>>,
}

impl RunDiagnosticRegistry {
    /// Adds a diagnostic to the run's ordered diagnostic history.
    ///
    /// # Parameters
    /// - `diagnostic`: Diagnostic to make available to application consumers.
    pub fn publish(&self, diagnostic: RunDiagnostic) {
        self.inner.write().unwrap().push(diagnostic);
    }

    /// Returns a snapshot of all diagnostics published so far.
    pub fn snapshot(&self) -> Vec<RunDiagnostic> {
        self.inner.read().unwrap().clone()
    }
}

/// Origin category of a source's preparation artifacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDataKind {
    /// Imported or persisted finite capture data.
    File,
    /// Data acquired from a live capture provider.
    Live,
}

/// Availability of one artifact exposed by source preparation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SourceArtifactReadiness {
    #[default]
    /// Artifact preparation has not completed.
    Pending,
    /// Artifact is available for application consumers.
    Available,
    /// The source does not support this artifact kind.
    Unsupported,
    /// Artifact preparation failed with a user-presentable error.
    Failed(String),
}

/// Readiness of source-owned artifacts needed by application consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceReadiness {
    /// Source node that owns these artifacts.
    pub source: NodeId,
    /// Whether the source is file-backed or live.
    pub kind: SourceDataKind,
    /// Readiness of preloaded source data.
    pub preload: SourceArtifactReadiness,
    /// Readiness of reusable prepared-data cache state.
    pub cache: SourceArtifactReadiness,
    /// Readiness of the waveform index.
    pub index: SourceArtifactReadiness,
    /// Readiness of source data delivered to the runtime.
    pub data: SourceArtifactReadiness,
}

/// Shared source/storage publication point retained by a run.
#[derive(Clone, Debug, Default)]
pub struct SourceReadinessRegistry {
    inner: Arc<RwLock<Vec<SourceReadiness>>>,
}

impl SourceReadinessRegistry {
    /// Publishes the latest readiness state for one source.
    ///
    /// Replaces prior state for the same source while preserving source ordering.
    pub fn publish(&self, readiness: SourceReadiness) {
        let mut entries = self.inner.write().unwrap();
        if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.source == readiness.source)
        {
            *existing = readiness;
        } else {
            entries.push(readiness);
            entries.sort_by_key(|entry| entry.source.0);
        }
    }

    /// Returns a sorted snapshot of every source's latest readiness state.
    pub fn snapshot(&self) -> Vec<SourceReadiness> {
        self.inner.read().unwrap().clone()
    }
}

#[cfg(test)]
mod run_data_tests {
    use super::*;
    use crate::CompileCtx;

    #[test]
    fn source_readiness_replaces_one_source_without_reordering_others() {
        let registry = SourceReadinessRegistry::default();
        let readiness = |source, data| SourceReadiness {
            source: NodeId(source),
            kind: SourceDataKind::Live,
            preload: SourceArtifactReadiness::Unsupported,
            cache: SourceArtifactReadiness::Available,
            index: SourceArtifactReadiness::Pending,
            data,
        };
        registry.publish(readiness(8, SourceArtifactReadiness::Pending));
        registry.publish(readiness(3, SourceArtifactReadiness::Available));
        registry.publish(readiness(8, SourceArtifactReadiness::Available));

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].source, NodeId(3));
        assert_eq!(snapshot[1].data, SourceArtifactReadiness::Available);
    }

    #[test]
    fn compile_context_snapshot_retains_shared_diagnostics_and_readiness() {
        let context = CompileCtx::default();
        let snapshot = context.run_data();
        context.diagnostics().publish(RunDiagnostic {
            node: Some(NodeId(4)),
            severity: RunDiagnosticSeverity::Warning,
            message: "cache is rebuilding".to_owned(),
        });
        context.source_readiness().publish(SourceReadiness {
            source: NodeId(4),
            kind: SourceDataKind::File,
            preload: SourceArtifactReadiness::Available,
            cache: SourceArtifactReadiness::Available,
            index: SourceArtifactReadiness::Pending,
            data: SourceArtifactReadiness::Pending,
        });

        assert_eq!(snapshot.diagnostics().snapshot().len(), 1);
        assert_eq!(snapshot.source_readiness().snapshot().len(), 1);
    }
}
