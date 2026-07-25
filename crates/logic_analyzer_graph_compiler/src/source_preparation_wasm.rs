use logic_analyzer_graph_api::node_support::CapturePresentation;

use super::{
    DiscoveredCapturePresentation, PreparedCapture, PreparedCaptureData, SourcePreparationStatus,
    SourcePreparationUpdate,
};

pub(crate) struct SourcePreparation {
    identity: Option<String>,
    status: SourcePreparationStatus,
}

impl SourcePreparation {
    pub(crate) fn new() -> Self {
        Self {
            identity: None,
            status: SourcePreparationStatus::Empty,
        }
    }

    pub(crate) fn synchronize(
        &mut self,
        discovered: Option<DiscoveredCapturePresentation>,
    ) -> SourcePreparationUpdate {
        let Some(discovered) = discovered else {
            let changed = self.identity.take().is_some();
            self.status = SourcePreparationStatus::Empty;
            return if changed {
                SourcePreparationUpdate::Cleared
            } else {
                SourcePreparationUpdate::Unchanged
            };
        };
        if self.identity.as_deref() == Some(discovered.identity.as_str()) {
            return SourcePreparationUpdate::Unchanged;
        }
        self.identity = Some(discovered.identity.clone());
        let data = match discovered.presentation {
            CapturePresentation::Indexed { .. } => {
                let error =
                    "filesystem-backed capture indexes are unavailable in this browser".to_owned();
                self.status = SourcePreparationStatus::Failed(error.clone());
                return SourcePreparationUpdate::Failed(error);
            }
            CapturePresentation::InMemory {
                signals,
                duration_us,
            } => PreparedCaptureData::InMemory {
                signals,
                duration_us,
            },
            CapturePresentation::Channels(channels) => PreparedCaptureData::Channels(channels),
        };
        self.status = SourcePreparationStatus::Ready;
        SourcePreparationUpdate::Ready(PreparedCapture {
            identity: discovered.identity,
            visible_channels: discovered.visible_channels,
            data,
        })
    }

    pub(crate) fn reset(&mut self) {
        self.identity = None;
        self.status = SourcePreparationStatus::Empty;
    }

    pub(crate) fn status(&self) -> SourcePreparationStatus {
        self.status.clone()
    }
}
