use std::sync::Arc;

use logic_analyzer_graph_plan::CollectedOutputSubscription;
use logic_analyzer_viewer::{
    DerivedLaneId, ViewerLaneBadge, ViewerLaneGroup, ViewerLaneGroupId, ViewerLaneRenderer,
    ViewerLaneTrack, WaveformPresentationRegistry, viewer_lane_renderer,
};

use crate::presentation_catalogs::PresentationBindingError;

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
) -> Result<(), PresentationBindingError> {
    for subscription in subscriptions {
        let mut pending_groups: Vec<PendingGroup> = Vec::new();
        for lane in &subscription.lanes {
            let lane_id = DerivedLaneId::new(lane.lane_name.clone());
            if let Some(presentation) = &lane.input.lane_presentation {
                let renderer =
                    viewer_lane_renderer(&presentation.renderer_key).ok_or_else(|| {
                        PresentationBindingError::UnknownLaneRenderer {
                            lane: lane.lane_name.clone(),
                            renderer: presentation.renderer_key.clone(),
                        }
                    })?;
                let [red, green, blue] = presentation.badge.color;
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
                        label: lane.source_label.clone(),
                        badge: ViewerLaneBadge::new(
                            presentation.badge.label.clone(),
                            egui::Color32::from_rgb(red, green, blue),
                        ),
                        renderer,
                        tracks: vec![(presentation.track_order, track)],
                    });
                }
            } else {
                let presentation =
                    lane.input
                        .default_lane_presentation
                        .as_ref()
                        .ok_or_else(|| {
                            PresentationBindingError::MissingDefaultLanePresentation {
                                payload_kind: lane.input.kind.name().to_owned(),
                            }
                        })?;
                registry.register(ViewerLaneGroup {
                    id: ViewerLaneGroupId::new(format!(
                        "{}:lane:{}",
                        subscription.runtime_name, lane.member
                    )),
                    label: lane.lane_name.clone(),
                    badge: {
                        let [red, green, blue] = presentation.badge.color;
                        ViewerLaneBadge::new(
                            presentation.badge.label.clone(),
                            egui::Color32::from_rgb(red, green, blue),
                        )
                    },
                    tracks: vec![ViewerLaneTrack::new("primary", lane_id, 1.0)],
                    renderer: viewer_lane_renderer(&presentation.renderer_key).ok_or_else(
                        || PresentationBindingError::UnknownLaneRenderer {
                            lane: lane.lane_name.clone(),
                            renderer: presentation.renderer_key.clone(),
                        },
                    )?,
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

pub(crate) fn waveform_presentation_registry(
    subscriptions: &[CollectedOutputSubscription],
) -> Result<WaveformPresentationRegistry, PresentationBindingError> {
    let registry = WaveformPresentationRegistry::new();
    registry.set_implicit_groups(false);
    bind_collected_output_presentations(&registry, subscriptions)?;
    Ok(registry)
}

#[cfg(test)]
mod collected_output_presentation_tests {
    use logic_analyzer_graph_capabilities::node_support::{
        LaneBadgeDescriptor, LanePresentationDescriptor, PortKind, ResolvedInput,
    };
    use logic_analyzer_graph_plan::CollectedOutputLane;
    use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerLaneRendererRegistration};
    use node_graph::NodeId;
    use signal_derived::Word;

    use super::*;

    const TEST_RENDERER: &str = "org.logicconduit.test.renderer.output/v1";

    inventory::submit! {
        ViewerLaneRendererRegistration::new(TEST_RENDERER, || Arc::new(DefaultViewerLaneRenderer))
    }

    fn grouped_lane(member: usize, track: &str, order: usize) -> CollectedOutputLane {
        CollectedOutputLane {
            member,
            lane_name: format!("Decoder.{track}"),
            source_label: "Decoder".to_owned(),
            input: ResolvedInput {
                kind: PortKind::of::<Word>(),
                source: format!("Decoder.{track}"),
                source_node: NodeId(7),
                source_output: member,
                source_node_title: "Decoder".to_owned(),
                source_output_title: track.to_owned(),
                word_display_format: None,
                lane_presentation: Some(LanePresentationDescriptor::new(
                    "frame",
                    track,
                    order,
                    1.0,
                    LaneBadgeDescriptor::new("W", [255, 255, 255]),
                    TEST_RENDERER,
                )),
                default_lane_presentation: None,
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

    #[test]
    fn ui_adapter_classifies_missing_default_presentations() {
        let mut lane = grouped_lane(0, "data", 0);
        lane.input.lane_presentation = None;
        let payload_kind = lane.input.kind.name().to_owned();
        let error = waveform_presentation_registry(&[CollectedOutputSubscription {
            runtime_name: "subscription".to_owned(),
            lanes: vec![lane],
        }])
        .err()
        .unwrap();

        assert_eq!(
            error,
            PresentationBindingError::MissingDefaultLanePresentation { payload_kind }
        );
    }

    #[test]
    fn ui_adapter_classifies_unknown_lane_renderers() {
        let mut lane = grouped_lane(0, "data", 0);
        lane.input.lane_presentation.as_mut().unwrap().renderer_key =
            "org.logicconduit.missing.renderer/v1".to_owned();
        let error = waveform_presentation_registry(&[CollectedOutputSubscription {
            runtime_name: "subscription".to_owned(),
            lanes: vec![lane],
        }])
        .err()
        .unwrap();

        assert_eq!(
            error,
            PresentationBindingError::UnknownLaneRenderer {
                lane: "Decoder.data".to_owned(),
                renderer: "org.logicconduit.missing.renderer/v1".to_owned(),
            }
        );
    }
}
