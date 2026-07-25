use std::sync::Arc;

use logic_analyzer_graph_api::node_support::{
    DecoderTableColumn, DecoderTableRegistry, DecoderTableSource,
};
use logic_analyzer_graph_compiler::CollectedTableSubscription;
use logic_analyzer_viewer::{DerivedLaneId, ViewerLaneTrackId};
use node_graph::NodeId;

struct PendingSource {
    source_node: NodeId,
    key: String,
    label: String,
    columns: Vec<(usize, DecoderTableColumn)>,
}

pub(crate) fn decoder_table_registry(
    subscriptions: &[CollectedTableSubscription],
) -> DecoderTableRegistry {
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
                renderer: Arc::clone(&table.renderer),
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
    registry
}

#[cfg(test)]
mod decoder_table_presentation_tests {
    use logic_analyzer_graph_api::node_support::{
        DecoderTableCellMode, DecoderTableColumnPresentation, PortKind, ResolvedInput,
    };
    use logic_analyzer_graph_compiler::CollectedOutputLane;
    use logic_analyzer_viewer::DefaultViewerLaneRenderer;
    use signal_processing::Word;

    use super::*;

    fn table_lane(member: usize, key: &str, order: usize) -> CollectedOutputLane {
        CollectedOutputLane {
            member,
            lane_name: format!("Decoder.{key}"),
            source_label: "Decoder".to_owned(),
            input: ResolvedInput {
                kind: PortKind::of::<Word>(),
                source: format!("Decoder.{key}"),
                source_node: NodeId(9),
                source_node_title: "Decoder".to_owned(),
                word_display_format: None,
                viewer_presentation: None,
                default_viewer_presentation: None,
                decoder_table_column: Some(DecoderTableColumnPresentation::new(
                    "frames",
                    key,
                    key.to_uppercase(),
                    order,
                    order == 0,
                    DecoderTableCellMode::Single,
                    key,
                    Arc::new(DefaultViewerLaneRenderer),
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
        }]);

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
}
