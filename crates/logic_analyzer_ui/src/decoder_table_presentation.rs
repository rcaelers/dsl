use logic_analyzer_graph_plan::CollectedTableSubscription;
use logic_analyzer_viewer::{DerivedLaneId, ViewerLaneTrackId, viewer_lane_renderer};
use node_graph::api::NodeId;

use crate::decoder_panel::{DecoderTableColumn, DecoderTableRegistry, DecoderTableSource};
use crate::presentation_catalogs::PresentationBindingError;

struct PendingSource {
    source_node: NodeId,
    key: String,
    label: String,
    columns: Vec<(usize, DecoderTableColumn)>,
}

pub(crate) fn decoder_table_registry(
    subscriptions: &[CollectedTableSubscription],
) -> Result<DecoderTableRegistry, PresentationBindingError> {
    let registry = DecoderTableRegistry::new();
    for subscription in subscriptions {
        let mut pending: Vec<PendingSource> = Vec::new();
        for lane in &subscription.lanes {
            let Some(table) = &lane.input.decoder_table_column else {
                continue;
            };
            let column = DecoderTableColumn {
                key: table.column_key.clone(),
                label: table.label.clone(),
                lane: DerivedLaneId::new(lane.lane_name.clone()),
                track: ViewerLaneTrackId::new(table.track_key.clone()),
                row_anchor: table.row_anchor,
                cell_mode: table.cell_mode.clone(),
                renderer: viewer_lane_renderer(&table.renderer_key).ok_or_else(|| {
                    PresentationBindingError::UnknownTableRenderer {
                        column: table.column_key.clone(),
                        renderer: table.renderer_key.clone(),
                    }
                })?,
            };
            if let Some(source) = pending.iter_mut().find(|source| {
                source.source_node == lane.input.source_node && source.key == table.source_key
            }) {
                source.columns.push((table.order, column));
            } else {
                pending.push(PendingSource {
                    source_node: lane.input.source_node,
                    key: table.source_key.clone(),
                    label: lane.input.source_node_title.clone(),
                    columns: vec![(table.order, column)],
                });
            }
        }
        for mut source in pending {
            source.columns.sort_by_key(|(order, _)| *order);
            registry.register(DecoderTableSource {
                id: format!(
                    "collector:{}:node:{}:{}",
                    subscription.collector.0, source.source_node.0, source.key
                ),
                label: source.label,
                columns: source
                    .columns
                    .into_iter()
                    .map(|(_, column)| column)
                    .collect(),
            });
        }
    }
    Ok(registry)
}

#[cfg(test)]
mod decoder_table_presentation_tests {
    use logic_analyzer_graph_capabilities::node_support::{
        DecoderTableCellMode, DecoderTableColumnDescriptor, PortKind, ResolvedInput,
    };
    use logic_analyzer_graph_plan::CollectedOutputLane;
    use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerLaneRendererRegistration};
    use signal_derived::Word;

    use super::*;

    const TEST_RENDERER: &str = "org.logicconduit.test.renderer.table/v1";

    inventory::submit! {
        ViewerLaneRendererRegistration::new(TEST_RENDERER, || {
            std::sync::Arc::new(DefaultViewerLaneRenderer)
        })
    }

    fn table_lane(member: usize, key: &str, order: usize) -> CollectedOutputLane {
        CollectedOutputLane {
            member,
            lane_name: format!("Decoder.{key}"),
            source_label: "Decoder".to_owned(),
            input: ResolvedInput {
                kind: PortKind::of::<Word>(),
                source: format!("Decoder.{key}"),
                source_node: NodeId(9),
                source_output: order,
                source_node_title: "Decoder".to_owned(),
                source_output_title: key.to_owned(),
                word_display_format: None,
                lane_presentation: None,
                default_lane_presentation: None,
                decoder_table_column: Some(DecoderTableColumnDescriptor::new(
                    "frames",
                    key,
                    key.to_uppercase(),
                    order,
                    order == 0,
                    DecoderTableCellMode::Single,
                    key,
                    TEST_RENDERER,
                )),
                capture_channel: None,
            },
        }
    }

    #[test]
    fn ui_adapter_groups_and_orders_decoder_columns() {
        let registry = decoder_table_registry(&[CollectedTableSubscription {
            collector: NodeId(4),
            lanes: vec![table_lane(1, "data", 1), table_lane(0, "bits", 0)],
        }])
        .unwrap();

        let sources = registry.read();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].label, "Decoder");
        assert_eq!(
            sources[0]
                .columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            ["bits", "data"]
        );
    }

    #[test]
    fn ui_adapter_classifies_unknown_table_renderers() {
        let mut lane = table_lane(0, "bits", 0);
        lane.input
            .decoder_table_column
            .as_mut()
            .unwrap()
            .renderer_key = "org.logicconduit.missing.table-renderer/v1".to_owned();
        let error = decoder_table_registry(&[CollectedTableSubscription {
            collector: NodeId(4),
            lanes: vec![lane],
        }])
        .err()
        .unwrap();

        assert_eq!(
            error,
            PresentationBindingError::UnknownTableRenderer {
                column: "bits".to_owned(),
                renderer: "org.logicconduit.missing.table-renderer/v1".to_owned(),
            }
        );
    }
}
