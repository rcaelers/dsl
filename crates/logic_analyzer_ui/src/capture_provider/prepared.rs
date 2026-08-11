use logic_analyzer_graph_runtime::{
    PreparedCaptureData, SourceDataKind, SourcePreparationStatus, SourcePreparationUpdate,
    SourceReadinessRegistry,
};
use node_graph::api::GraphState;

use super::contract::{
    CaptureArtifactUpdate, CaptureDataProvider, CapturePresentationUpdate, CaptureProviderError,
    CaptureProviderPoll, CaptureReadinessUpdate,
};
use crate::graph_service::UiGraphService;

/// Adapts finite source preparation to the UI-owned provider port.
pub(crate) struct PreparedCaptureProvider<'a> {
    service: &'a mut UiGraphService,
    graph: &'a GraphState,
    readiness: Option<SourceReadinessRegistry>,
}

impl<'a> PreparedCaptureProvider<'a> {
    pub(crate) fn new(
        service: &'a mut UiGraphService,
        graph: &'a GraphState,
        readiness: Option<SourceReadinessRegistry>,
    ) -> Self {
        Self {
            service,
            graph,
            readiness,
        }
    }
}

impl CaptureDataProvider for PreparedCaptureProvider<'_> {
    fn poll(&mut self) -> CaptureProviderPoll {
        let update = self.service.synchronize_prepared_capture(self.graph);
        let status = self.service.source_preparation_status();
        let poll_again = matches!(status, SourcePreparationStatus::Preparing);
        let presentation = match update {
            SourcePreparationUpdate::Unchanged => CapturePresentationUpdate::Unchanged,
            SourcePreparationUpdate::Cleared => CapturePresentationUpdate::Clear {
                restore_prepared: false,
            },
            SourcePreparationUpdate::Preparing(preparing) => CapturePresentationUpdate::Preparing {
                identity: preparing.identity,
                visible_channels: preparing.visible_channels,
                metadata: preparing.metadata,
                progress: preparing.progress,
            },
            SourcePreparationUpdate::Ready(prepared) => match prepared.data {
                PreparedCaptureData::Indexed(index) => CapturePresentationUpdate::Indexed {
                    identity: prepared.identity,
                    visible_channels: Some(prepared.visible_channels),
                    index,
                    growing: false,
                    planned_span_us: None,
                },
                PreparedCaptureData::InMemory {
                    signals,
                    duration_us,
                } => CapturePresentationUpdate::InMemory {
                    identity: prepared.identity,
                    visible_channels: prepared.visible_channels,
                    signals,
                    duration_us,
                },
                PreparedCaptureData::Channels(channels) => CapturePresentationUpdate::Channels {
                    identity: prepared.identity,
                    visible_channels: prepared.visible_channels,
                    channels,
                },
            },
            SourcePreparationUpdate::Failed(error) => {
                CapturePresentationUpdate::Failed(CaptureProviderError::Preparation(error))
            }
        };
        let readiness = self.readiness.clone().and_then(|registry| match status {
            SourcePreparationStatus::Ready => Some(
                CaptureReadinessUpdate::new(registry, SourceDataKind::File)
                    .with_preload(CaptureArtifactUpdate::AvailableIfPending)
                    .with_cache(CaptureArtifactUpdate::AvailableIfPending)
                    .with_index(CaptureArtifactUpdate::AvailableIfPending)
                    .with_data(CaptureArtifactUpdate::AvailableIfPending),
            ),
            SourcePreparationStatus::Failed(error) => {
                let failure = CaptureArtifactUpdate::Failed(error.to_string());
                Some(
                    CaptureReadinessUpdate::new(registry, SourceDataKind::File)
                        .with_preload(failure.clone())
                        .with_cache(failure.clone())
                        .with_index(failure.clone())
                        .with_data(failure),
                )
            }
            SourcePreparationStatus::Empty | SourcePreparationStatus::Preparing => None,
        });
        CaptureProviderPoll {
            presentation,
            readiness,
            poll_again,
        }
    }
}
