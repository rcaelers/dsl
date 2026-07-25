use logic_analyzer_graph_compiler::ResolvedSamplingOverlay;
use logic_analyzer_viewer::{SamplingOverlay, SamplingQualifier};

pub(crate) fn sampling_overlay_presentation(resolved: &ResolvedSamplingOverlay) -> SamplingOverlay {
    SamplingOverlay {
        clock_channel: resolved.clock_channel,
        sampled_channels: resolved.sampled_channels.clone(),
        edge: resolved.edge,
        qualifiers: resolved
            .qualifiers
            .iter()
            .map(|qualifier| SamplingQualifier {
                channel: qualifier.channel,
                active_level: qualifier.active_level,
            })
            .collect(),
        activities: resolved.activities.clone(),
    }
}

#[cfg(test)]
mod sampling_overlay_presentation_tests {
    use logic_analyzer_graph_compiler::ResolvedSamplingQualifier;
    use signal_processing::{SamplingActivity, SamplingEdge};

    use super::*;

    #[test]
    fn ui_adapter_preserves_resolved_sampling_contract() {
        let activity = SamplingActivity::default();
        let resolved = ResolvedSamplingOverlay {
            clock_channel: 3,
            sampled_channels: vec![1, 2],
            edge: SamplingEdge::Falling,
            qualifiers: vec![ResolvedSamplingQualifier {
                channel: 4,
                active_level: false,
            }],
            activities: vec![activity.clone()],
        };

        let overlay = sampling_overlay_presentation(&resolved);

        assert_eq!(overlay.clock_channel, 3);
        assert_eq!(overlay.sampled_channels, [1, 2]);
        assert_eq!(overlay.edge, SamplingEdge::Falling);
        assert_eq!(overlay.qualifiers[0].channel, 4);
        assert!(!overlay.qualifiers[0].active_level);
        assert_eq!(overlay.activities.len(), 1);
    }
}
