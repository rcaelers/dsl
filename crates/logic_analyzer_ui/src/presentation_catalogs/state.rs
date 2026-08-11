use std::collections::HashSet;

use logic_analyzer_graph_plan as plan;
use node_graph::api::{GraphState, NodeId};

use crate::app::SavedViewerRow;
use crate::decoder_panel::{DecoderPanels, DecoderTableRegistry};
use crate::plugin_panel::{PluginPanelRegistry, PluginPanels};
use crate::viewer_selection::viewer_output_selections;

/// Owns presentation data whose members must advance and clear together.
///
/// Output and table catalogs describe the same presented derived-lane snapshot. Decoder and plugin
/// panels are rebound whenever that snapshot changes. Saved selections are retained independently
/// so a temporarily absent lane or overlay can reappear without losing the user's choice.
pub(crate) struct PresentationCatalogs {
    presented_derived_lanes: signal_derived::DerivedLanes,
    output_catalog: Vec<plan::CollectedOutputSubscription>,
    table_catalog: Vec<plan::CollectedTableSubscription>,
    graph_nodes: HashSet<NodeId>,
    decoder_panels: DecoderPanels,
    plugin_panels: PluginPanels,
    viewer_lane_order: Vec<SavedViewerRow>,
    selected_sampling_overlays: Vec<NodeId>,
}

impl PresentationCatalogs {
    pub(crate) fn new(
        plugin_panel_registry: PluginPanelRegistry,
        graph_nodes: HashSet<NodeId>,
    ) -> Self {
        Self {
            presented_derived_lanes: signal_derived::DerivedLanes::new(),
            output_catalog: Vec::new(),
            table_catalog: Vec::new(),
            graph_nodes,
            decoder_panels: DecoderPanels::default(),
            plugin_panels: PluginPanels::new(plugin_panel_registry),
            viewer_lane_order: Vec::new(),
            selected_sampling_overlays: Vec::new(),
        }
    }

    pub(crate) fn presented_derived_lanes(&self) -> &signal_derived::DerivedLanes {
        &self.presented_derived_lanes
    }

    pub(crate) fn set_presented_derived_lanes(&mut self, lanes: signal_derived::DerivedLanes) {
        self.presented_derived_lanes = lanes;
    }

    pub(crate) fn replace_run_catalogs(
        &mut self,
        run_data: &logic_analyzer_graph_runtime::RunData,
    ) {
        self.presented_derived_lanes = run_data.derived_lanes().clone();
        self.output_catalog = run_data.output_subscriptions().to_vec();
        self.table_catalog = run_data.table_subscriptions().to_vec();
        self.plugin_panels
            .set_run_data(self.presented_derived_lanes.clone());
    }

    #[cfg(test)]
    pub(crate) fn replace_catalogs(
        &mut self,
        outputs: Vec<plan::CollectedOutputSubscription>,
        tables: Vec<plan::CollectedTableSubscription>,
    ) {
        self.output_catalog = outputs;
        self.table_catalog = tables;
    }

    pub(crate) fn clear_run_catalogs(&mut self) -> signal_derived::DerivedLanes {
        self.presented_derived_lanes = signal_derived::DerivedLanes::new();
        self.output_catalog.clear();
        self.table_catalog.clear();
        self.decoder_panels.set_run_data(
            self.presented_derived_lanes.clone(),
            DecoderTableRegistry::new(),
        );
        self.plugin_panels
            .set_run_data(self.presented_derived_lanes.clone());
        self.presented_derived_lanes.clone()
    }

    pub(crate) fn visible_output_subscriptions(
        &self,
        graph: &GraphState,
    ) -> Vec<plan::CollectedOutputSubscription> {
        let selected = viewer_output_selections(graph)
            .into_iter()
            .filter(|selection| selection.selected)
            .map(|selection| (selection.node, selection.output))
            .collect::<HashSet<_>>();
        self.output_catalog
            .iter()
            .filter_map(|subscription| {
                let mut subscription = subscription.clone();
                subscription.lanes.retain(|lane| {
                    selected.contains(&(lane.input.source_node, lane.input.source_output))
                });
                (!subscription.lanes.is_empty()).then_some(subscription)
            })
            .collect()
    }

    pub(crate) fn visible_table_subscriptions(
        &self,
        graph: &GraphState,
    ) -> Vec<plan::CollectedTableSubscription> {
        self.table_catalog
            .iter()
            .filter_map(|subscription| {
                let mut subscription = subscription.clone();
                subscription
                    .lanes
                    .retain(|lane| graph.nodes.contains_key(&lane.input.source_node));
                (!subscription.lanes.is_empty()).then_some(subscription)
            })
            .collect()
    }

    pub(crate) fn merge_run_catalogs(
        &mut self,
        outputs: &[plan::CollectedOutputSubscription],
        tables: &[plan::CollectedTableSubscription],
    ) {
        merge_output_subscription_catalog(&mut self.output_catalog, outputs);
        merge_table_subscription_catalog(&mut self.table_catalog, tables);
    }

    pub(crate) fn synchronize_graph_nodes(&mut self, graph: &GraphState) -> bool {
        let graph_nodes = graph.nodes.keys().copied().collect();
        if self.graph_nodes == graph_nodes {
            return false;
        }
        self.graph_nodes = graph_nodes;
        true
    }

    pub(crate) fn decoder_panels(&self) -> &DecoderPanels {
        &self.decoder_panels
    }

    pub(crate) fn decoder_panels_mut(&mut self) -> &mut DecoderPanels {
        &mut self.decoder_panels
    }

    pub(crate) fn replace_decoder_panels(&mut self, panels: DecoderPanels) {
        self.decoder_panels = panels;
    }

    pub(crate) fn plugin_panels(&self) -> &PluginPanels {
        &self.plugin_panels
    }

    pub(crate) fn plugin_panels_mut(&mut self) -> &mut PluginPanels {
        &mut self.plugin_panels
    }

    pub(crate) fn viewer_lane_order(&self) -> &[SavedViewerRow] {
        &self.viewer_lane_order
    }

    pub(crate) fn replace_viewer_lane_order(&mut self, order: Vec<SavedViewerRow>) {
        self.viewer_lane_order = order;
    }

    pub(crate) fn clear_viewer_lane_order(&mut self) {
        self.viewer_lane_order.clear();
    }

    pub(crate) fn selected_sampling_overlays(&self) -> &[NodeId] {
        &self.selected_sampling_overlays
    }

    pub(crate) fn replace_selected_sampling_overlays(&mut self, selected: Vec<NodeId>) {
        self.selected_sampling_overlays = selected;
    }

    pub(crate) fn clear_selected_sampling_overlays(&mut self) {
        self.selected_sampling_overlays.clear();
    }

    pub(crate) fn retain_sampling_overlay_candidates(
        &mut self,
        candidates: &[plan::SamplingOverlayCandidate],
    ) -> bool {
        let previous_len = self.selected_sampling_overlays.len();
        self.selected_sampling_overlays.retain(|selected| {
            candidates
                .iter()
                .any(|candidate| candidate.node_id() == *selected)
        });
        self.selected_sampling_overlays.len() != previous_len
    }

    pub(crate) fn toggle_sampling_overlay(&mut self, node: NodeId) {
        if let Some(index) = self
            .selected_sampling_overlays
            .iter()
            .position(|candidate| *candidate == node)
        {
            self.selected_sampling_overlays.remove(index);
        } else {
            self.selected_sampling_overlays.push(node);
        }
    }
}

fn merge_output_subscription_catalog(
    catalog: &mut Vec<plan::CollectedOutputSubscription>,
    incoming: &[plan::CollectedOutputSubscription],
) {
    for subscription in incoming {
        for lane in &subscription.lanes {
            for existing in catalog.iter_mut() {
                existing.lanes.retain(|candidate| {
                    candidate.input.source_node != lane.input.source_node
                        || candidate.input.source_output != lane.input.source_output
                });
            }
        }
        let target = if let Some(existing) = catalog
            .iter_mut()
            .find(|existing| existing.runtime_name == subscription.runtime_name)
        {
            existing
        } else {
            catalog.push(plan::CollectedOutputSubscription {
                runtime_name: subscription.runtime_name.clone(),
                lanes: Vec::new(),
            });
            catalog.last_mut().expect("catalog entry was just inserted")
        };
        target.lanes.extend(subscription.lanes.iter().cloned());
    }
    catalog.retain(|subscription| !subscription.lanes.is_empty());
}

fn merge_table_subscription_catalog(
    catalog: &mut Vec<plan::CollectedTableSubscription>,
    incoming: &[plan::CollectedTableSubscription],
) {
    for subscription in incoming {
        for lane in &subscription.lanes {
            for existing in catalog.iter_mut() {
                existing.lanes.retain(|candidate| {
                    candidate.input.source_node != lane.input.source_node
                        || candidate.input.source_output != lane.input.source_output
                });
            }
        }
        let target = if let Some(existing) = catalog
            .iter_mut()
            .find(|existing| existing.collector == subscription.collector)
        {
            existing
        } else {
            catalog.push(plan::CollectedTableSubscription {
                collector: subscription.collector,
                lanes: Vec::new(),
            });
            catalog.last_mut().expect("catalog entry was just inserted")
        };
        target.lanes.extend(subscription.lanes.iter().cloned());
    }
    catalog.retain(|subscription| !subscription.lanes.is_empty());
}
