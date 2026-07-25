use std::sync::{Arc, RwLock};

use node_graph::NodeId;
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

    pub fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    pub fn output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.output_subscriptions
    }

    pub fn table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.table_subscriptions
    }

    pub fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        &self.sampling_overlays
    }

    pub fn diagnostics(&self) -> &RunDiagnosticRegistry {
        &self.diagnostics
    }

    pub fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDiagnostic {
    pub node: Option<NodeId>,
    pub severity: RunDiagnosticSeverity,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct RunDiagnosticRegistry {
    inner: Arc<RwLock<Vec<RunDiagnostic>>>,
}

impl RunDiagnosticRegistry {
    pub fn publish(&self, diagnostic: RunDiagnostic) {
        self.inner.write().unwrap().push(diagnostic);
    }

    pub fn snapshot(&self) -> Vec<RunDiagnostic> {
        self.inner.read().unwrap().clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDataKind {
    File,
    Live,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SourceArtifactReadiness {
    #[default]
    Pending,
    Available,
    Unsupported,
    Failed(String),
}

/// Readiness of source-owned artifacts needed by application consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceReadiness {
    pub source: NodeId,
    pub kind: SourceDataKind,
    pub preload: SourceArtifactReadiness,
    pub cache: SourceArtifactReadiness,
    pub index: SourceArtifactReadiness,
    pub data: SourceArtifactReadiness,
}

/// Shared source/storage publication point retained by a run.
#[derive(Clone, Debug, Default)]
pub struct SourceReadinessRegistry {
    inner: Arc<RwLock<Vec<SourceReadiness>>>,
}

impl SourceReadinessRegistry {
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
