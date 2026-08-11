use logic_analyzer_graph_runtime::{SourceDataKind, SourceReadinessRegistry};

use super::contract::{
    CaptureArtifactUpdate, CaptureDataProvider, CapturePresentationUpdate, CaptureProviderPoll,
    CaptureReadinessUpdate,
};
use crate::live_capture::CaptureCoordinatorContract;

/// Adapts one active acquisition coordinator to the UI-owned provider port.
pub(crate) struct LiveCaptureProvider<'a> {
    acquisition: &'a mut dyn CaptureCoordinatorContract,
    readiness: Option<SourceReadinessRegistry>,
}

impl<'a> LiveCaptureProvider<'a> {
    pub(crate) fn new(
        acquisition: &'a mut dyn CaptureCoordinatorContract,
        readiness: Option<SourceReadinessRegistry>,
    ) -> Self {
        Self {
            acquisition,
            readiness,
        }
    }

    pub(crate) fn analysis_ready(readiness: SourceReadinessRegistry) -> CaptureReadinessUpdate {
        CaptureReadinessUpdate::new(readiness, SourceDataKind::Live)
            .with_cache(CaptureArtifactUpdate::Available)
            .with_data(CaptureArtifactUpdate::Available)
    }
}

impl CaptureDataProvider for LiveCaptureProvider<'_> {
    fn poll(&mut self) -> CaptureProviderPoll {
        let planned_span_us = self
            .acquisition
            .status()
            .and_then(|status| status.session_plan.as_ref())
            .and_then(planned_waveform_span_us);
        let identity = self.acquisition.status().map_or_else(
            || "Live capture".to_owned(),
            |status| status.source_title.clone(),
        );
        let Some(update) = self.acquisition.take_waveform_update() else {
            return CaptureProviderPoll::unchanged();
        };
        let Some(index) = update else {
            return CaptureProviderPoll {
                presentation: CapturePresentationUpdate::Clear {
                    restore_prepared: true,
                },
                readiness: None,
                poll_again: false,
            };
        };
        let readiness = self.readiness.clone().map(|registry| {
            CaptureReadinessUpdate::new(registry, SourceDataKind::Live)
                .with_cache(CaptureArtifactUpdate::Available)
                .with_index(CaptureArtifactUpdate::Available)
                .with_data(CaptureArtifactUpdate::Available)
        });
        CaptureProviderPoll {
            presentation: CapturePresentationUpdate::Indexed {
                identity,
                visible_channels: None,
                index,
                growing: true,
                planned_span_us,
            },
            readiness,
            poll_again: false,
        }
    }

    fn acquisition(&mut self) -> Option<&mut dyn CaptureCoordinatorContract> {
        Some(self.acquisition)
    }
}

fn planned_waveform_span_us(plan: &signal_capture_session::CaptureSessionPlan) -> Option<f64> {
    let samples = plan.capture_window_samples?;
    if plan.sample_rate_hz == 0 {
        return None;
    }
    Some(samples as f64 * 1_000_000.0 / plan.sample_rate_hz as f64)
}
