use logic_analyzer_graph_compiler::ResolvedSamplingOverlay;
use logic_analyzer_viewer::SamplingOverlay;

pub(crate) fn sampling_overlay_presentation(resolved: &ResolvedSamplingOverlay) -> SamplingOverlay {
    SamplingOverlay {
        clock_channel: resolved.clock_channel,
        sampled_channels: resolved.sampled_channels.clone(),
        points: resolved.points.clone(),
    }
}

#[cfg(test)]
mod sampling_overlay_presentation_tests {
    use signal_processing::{SamplingPoint, SamplingPointStore};

    use super::*;

    #[test]
    fn ui_adapter_preserves_resolved_sampling_contract() {
        let points = SamplingPointStore::default();
        points.record(SamplingPoint::new(12, true, vec![false, true]));
        let resolved = ResolvedSamplingOverlay {
            clock_channel: 3,
            sampled_channels: vec![1, 2],
            points,
        };

        let overlay = sampling_overlay_presentation(&resolved);

        assert_eq!(overlay.clock_channel, 3);
        assert_eq!(overlay.sampled_channels, [1, 2]);
        assert_eq!(
            overlay.points.points_in_range(0, u64::MAX),
            [SamplingPoint::new(12, true, vec![false, true])]
        );
    }
}
