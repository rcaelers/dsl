use logic_analyzer_graph_capabilities::node_support::CapturePresentationSignal;
use logic_analyzer_graph_runtime::{
    SourceArtifactReadiness, SourceDataKind, SourcePreparationError, SourceReadinessRegistry,
};
use signal_capture::{CaptureIndex, CaptureIndexBuildProgress, CaptureMetadata};

use crate::live_capture::CaptureCoordinatorContract;

/// Presentation change published by a provider without exposing how its data was obtained.
pub(crate) enum CapturePresentationUpdate {
    Unchanged,
    Clear {
        restore_prepared: bool,
    },
    Preparing {
        identity: String,
        visible_channels: Vec<usize>,
        metadata: Option<CaptureMetadata>,
        progress: Option<CaptureIndexBuildProgress>,
    },
    Indexed {
        identity: String,
        visible_channels: Option<Vec<usize>>,
        index: Box<dyn CaptureIndex>,
        growing: bool,
        planned_span_us: Option<f64>,
    },
    InMemory {
        identity: String,
        visible_channels: Vec<usize>,
        signals: Vec<CapturePresentationSignal>,
        duration_us: f64,
    },
    Channels {
        identity: String,
        visible_channels: Vec<usize>,
        channels: Vec<(usize, String)>,
    },
    Failed(CaptureProviderError),
}

/// UI boundary failure reported while a provider prepares presentable data.
#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum CaptureProviderError {
    #[error("could not prepare capture source: {0}")]
    Preparation(#[source] SourcePreparationError),
}

/// One artifact readiness transition applied to matching source records.
#[derive(Clone, Debug)]
pub(crate) enum CaptureArtifactUpdate {
    Preserve,
    Available,
    AvailableIfPending,
    Failed(String),
}

/// Provider-authored readiness changes for one explicit source category.
pub(crate) struct CaptureReadinessUpdate {
    registry: SourceReadinessRegistry,
    source_kind: SourceDataKind,
    preload: CaptureArtifactUpdate,
    cache: CaptureArtifactUpdate,
    index: CaptureArtifactUpdate,
    data: CaptureArtifactUpdate,
}

impl CaptureReadinessUpdate {
    pub(crate) fn new(registry: SourceReadinessRegistry, source_kind: SourceDataKind) -> Self {
        Self {
            registry,
            source_kind,
            preload: CaptureArtifactUpdate::Preserve,
            cache: CaptureArtifactUpdate::Preserve,
            index: CaptureArtifactUpdate::Preserve,
            data: CaptureArtifactUpdate::Preserve,
        }
    }

    pub(crate) fn with_preload(mut self, update: CaptureArtifactUpdate) -> Self {
        self.preload = update;
        self
    }

    pub(crate) fn with_cache(mut self, update: CaptureArtifactUpdate) -> Self {
        self.cache = update;
        self
    }

    pub(crate) fn with_index(mut self, update: CaptureArtifactUpdate) -> Self {
        self.index = update;
        self
    }

    pub(crate) fn with_data(mut self, update: CaptureArtifactUpdate) -> Self {
        self.data = update;
        self
    }

    pub(crate) fn publish(self) {
        for mut readiness in self
            .registry
            .snapshot()
            .into_iter()
            .filter(|readiness| readiness.kind == self.source_kind)
        {
            apply_artifact_update(&mut readiness.preload, &self.preload);
            apply_artifact_update(&mut readiness.cache, &self.cache);
            apply_artifact_update(&mut readiness.index, &self.index);
            apply_artifact_update(&mut readiness.data, &self.data);
            self.registry.publish(readiness);
        }
    }
}

fn apply_artifact_update(readiness: &mut SourceArtifactReadiness, update: &CaptureArtifactUpdate) {
    match update {
        CaptureArtifactUpdate::Preserve => {}
        CaptureArtifactUpdate::Available => *readiness = SourceArtifactReadiness::Available,
        CaptureArtifactUpdate::AvailableIfPending => {
            if *readiness == SourceArtifactReadiness::Pending {
                *readiness = SourceArtifactReadiness::Available;
            }
        }
        CaptureArtifactUpdate::Failed(message) => {
            if *readiness != SourceArtifactReadiness::Unsupported {
                *readiness = SourceArtifactReadiness::Failed(message.clone());
            }
        }
    }
}

/// One provider poll result consumed atomically by the application shell.
pub(crate) struct CaptureProviderPoll {
    pub(crate) presentation: CapturePresentationUpdate,
    pub(crate) readiness: Option<CaptureReadinessUpdate>,
    pub(crate) poll_again: bool,
}

impl CaptureProviderPoll {
    pub(crate) fn unchanged() -> Self {
        Self {
            presentation: CapturePresentationUpdate::Unchanged,
            readiness: None,
            poll_again: false,
        }
    }
}

/// UI-owned data-provider port shared by prepared and acquiring capture sources.
pub(crate) trait CaptureDataProvider {
    fn poll(&mut self) -> CaptureProviderPoll;

    fn acquisition(&mut self) -> Option<&mut dyn CaptureCoordinatorContract> {
        None
    }
}

#[cfg(test)]
mod contract_tests {
    use logic_analyzer_graph_runtime::{SourceReadiness, SourceReadinessRegistry};
    use node_graph::NodeId;

    use super::*;

    fn readiness(source: u32, kind: SourceDataKind) -> SourceReadiness {
        SourceReadiness {
            source: NodeId(source),
            kind,
            preload: SourceArtifactReadiness::Pending,
            cache: SourceArtifactReadiness::Unsupported,
            index: SourceArtifactReadiness::Pending,
            data: SourceArtifactReadiness::Pending,
        }
    }

    #[test]
    fn readiness_updates_only_the_explicit_provider_category() {
        let registry = SourceReadinessRegistry::default();
        registry.publish(readiness(1, SourceDataKind::File));
        registry.publish(readiness(2, SourceDataKind::Live));

        CaptureReadinessUpdate::new(registry.clone(), SourceDataKind::File)
            .with_preload(CaptureArtifactUpdate::AvailableIfPending)
            .with_cache(CaptureArtifactUpdate::AvailableIfPending)
            .with_index(CaptureArtifactUpdate::AvailableIfPending)
            .with_data(CaptureArtifactUpdate::AvailableIfPending)
            .publish();

        let snapshot = registry.snapshot();
        let file = &snapshot[0];
        assert_eq!(file.preload, SourceArtifactReadiness::Available);
        assert_eq!(file.cache, SourceArtifactReadiness::Unsupported);
        assert_eq!(file.index, SourceArtifactReadiness::Available);
        assert_eq!(file.data, SourceArtifactReadiness::Available);
        assert_eq!(snapshot[1], readiness(2, SourceDataKind::Live));
    }

    #[test]
    fn failed_readiness_preserves_unsupported_artifacts() {
        let registry = SourceReadinessRegistry::default();
        registry.publish(readiness(1, SourceDataKind::File));
        let failed = CaptureArtifactUpdate::Failed("could not read capture".into());

        CaptureReadinessUpdate::new(registry.clone(), SourceDataKind::File)
            .with_preload(failed.clone())
            .with_cache(failed.clone())
            .with_index(failed.clone())
            .with_data(failed)
            .publish();

        let file = registry.snapshot().remove(0);
        assert!(matches!(file.preload, SourceArtifactReadiness::Failed(_)));
        assert_eq!(file.cache, SourceArtifactReadiness::Unsupported);
        assert!(matches!(file.index, SourceArtifactReadiness::Failed(_)));
        assert!(matches!(file.data, SourceArtifactReadiness::Failed(_)));
    }
}
