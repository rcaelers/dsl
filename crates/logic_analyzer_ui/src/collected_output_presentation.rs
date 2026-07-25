use std::sync::Arc;

use logic_analyzer_graph_compiler::CollectedOutputSubscription;
use logic_analyzer_viewer::{
    DerivedLaneId, ViewerLaneBadge, ViewerLaneGroup, ViewerLaneGroupId, ViewerLaneRenderer,
    ViewerLaneTrack, WaveformPresentationRegistry,
};

struct PendingGroup {
    source_node: node_graph::NodeId,
    key: String,
    label: String,
    badge: ViewerLaneBadge,
    renderer: Arc<dyn ViewerLaneRenderer>,
    tracks: Vec<(usize, ViewerLaneTrack)>,
}

pub(crate) fn bind_collected_output_presentations(
    registry: &WaveformPresentationRegistry,
    subscriptions: &[CollectedOutputSubscription],
) -> Result<(), String> {
    for subscription in subscriptions {
        let mut pending_groups: Vec<PendingGroup> = Vec::new();
        for lane in &subscription.lanes {
            let lane_id = DerivedLaneId::new(lane.lane_name.clone());
            if let Some(presentation) = &lane.input.viewer_presentation {
                let track = ViewerLaneTrack::new(
                    presentation.track_key.clone(),
                    lane_id,
                    presentation.relative_height,
                );
                if let Some(group) = pending_groups.iter_mut().find(|group| {
                    group.source_node == lane.input.source_node
                        && group.key == presentation.group_key
                }) {
                    group.tracks.push((presentation.track_order, track));
                } else {
                    pending_groups.push(PendingGroup {
                        source_node: lane.input.source_node,
                        key: presentation.group_key.clone(),
                        label: lane.input.source_node_title.clone(),
                        badge: presentation.badge.clone(),
                        renderer: Arc::clone(&presentation.renderer),
                        tracks: vec![(presentation.track_order, track)],
                    });
                }
            } else {
                let presentation =
                    lane.input
                        .default_viewer_presentation
                        .as_ref()
                        .ok_or_else(|| {
                            format!(
                                "subscribed payload '{}' has no default presentation",
                                lane.input.kind.name()
                            )
                        })?;
                registry.register(ViewerLaneGroup {
                    id: ViewerLaneGroupId::new(format!(
                        "{}:lane:{}",
                        subscription.runtime_name, lane.member
                    )),
                    label: lane.lane_name.clone(),
                    badge: presentation.badge().clone(),
                    tracks: vec![ViewerLaneTrack::new("primary", lane_id, 1.0)],
                    renderer: presentation.renderer(),
                });
            }
        }
        for mut pending in pending_groups {
            pending.tracks.sort_by_key(|(order, _)| *order);
            registry.register(ViewerLaneGroup {
                id: ViewerLaneGroupId::new(format!(
                    "{}:node:{}:{}",
                    subscription.runtime_name, pending.source_node.0, pending.key
                )),
                label: pending.label,
                badge: pending.badge,
                tracks: pending.tracks.into_iter().map(|(_, track)| track).collect(),
                renderer: pending.renderer,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod collected_output_presentation_tests {
    use logic_analyzer_graph_api::node_support::{PortKind, ResolvedInput};
    use logic_analyzer_graph_compiler::CollectedOutputLane;
    use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerOutputPresentation};
    use node_graph::NodeId;
    use signal_processing::Word;

    use super::*;

    fn grouped_lane(member: usize, track: &str, order: usize) -> CollectedOutputLane {
        CollectedOutputLane {
            member,
            lane_name: format!("Decoder.{track}"),
            input: ResolvedInput {
                kind: PortKind::of::<Word>(),
                source: format!("Decoder.{track}"),
                source_node: NodeId(7),
                source_node_title: "Decoder".to_owned(),
                word_display_format: None,
                viewer_presentation: Some(ViewerOutputPresentation::new(
                    "frame",
                    track,
                    order,
                    1.0,
                    ViewerLaneBadge::new("W", egui::Color32::WHITE),
                    Arc::new(DefaultViewerLaneRenderer),
                )),
                default_viewer_presentation: None,
                decoder_table_column: None,
                capture_channel: None,
            },
        }
    }

    #[test]
    fn ui_adapter_groups_and_orders_collected_tracks() {
        let registry = WaveformPresentationRegistry::new();
        let subscriptions = [CollectedOutputSubscription {
            runtime_name: "subscription".to_owned(),
            lanes: vec![grouped_lane(1, "data", 1), grouped_lane(0, "bits", 0)],
        }];

        bind_collected_output_presentations(&registry, &subscriptions).unwrap();

        let groups = registry.read();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Decoder");
        assert_eq!(
            groups[0]
                .tracks
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["bits", "data"]
        );
    }
}
