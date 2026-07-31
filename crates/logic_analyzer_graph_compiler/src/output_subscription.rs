use logic_analyzer_graph_api::node_support::ResolvedInput;
use node_graph::api::NodeId;

/// Application-supplied retained outputs and the subset currently presented.
///
/// Retention affects runtime collection. Visibility only selects metadata for
/// consumers of already-retained lanes; changing it does not alter execution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputSubscriptionPlan {
    visible_outputs: Vec<(NodeId, usize)>,
    retained_outputs: Vec<(NodeId, usize)>,
    sampling_overlays: Vec<NodeId>,
}

/// One retained lane produced for an application output subscription.
#[derive(Clone, Debug)]
pub struct CollectedOutputLane {
    pub member: usize,
    pub lane_name: String,
    pub source_label: String,
    pub input: ResolvedInput,
}

/// Runtime identities and source metadata for one collected output set.
#[derive(Clone, Debug)]
pub struct CollectedOutputSubscription {
    pub runtime_name: String,
    pub lanes: Vec<CollectedOutputLane>,
}

/// Retained lanes carrying decoder-table column metadata for one collector.
#[derive(Clone, Debug)]
pub struct CollectedTableSubscription {
    pub collector: NodeId,
    pub lanes: Vec<CollectedOutputLane>,
}

impl OutputSubscriptionPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&mut self, node: NodeId, output: usize) {
        self.retain(node, output);
        if !self.contains(node, output) {
            self.visible_outputs.push((node, output));
        }
    }

    /// Retains an endpoint for later consumers without making it visible.
    pub fn retain(&mut self, node: NodeId, output: usize) {
        if !self.is_retained(node, output) {
            self.retained_outputs.push((node, output));
        }
    }

    pub fn contains(&self, node: NodeId, output: usize) -> bool {
        self.visible_outputs.contains(&(node, output))
    }

    pub fn is_retained(&self, node: NodeId, output: usize) -> bool {
        self.retained_outputs.contains(&(node, output))
    }

    pub fn outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.visible_outputs.iter().copied()
    }

    pub fn retained_outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.retained_outputs.iter().copied()
    }

    /// Collects sampling decisions for one application-visible overlay.
    pub fn subscribe_sampling_overlay(&mut self, node: NodeId) {
        if !self.collects_sampling_overlay(node) {
            self.sampling_overlays.push(node);
        }
    }

    pub fn collects_sampling_overlay(&self, node: NodeId) -> bool {
        self.sampling_overlays.contains(&node)
    }

    pub fn sampling_overlays(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.sampling_overlays.iter().copied()
    }
}

impl FromIterator<(NodeId, usize)> for OutputSubscriptionPlan {
    fn from_iter<T: IntoIterator<Item = (NodeId, usize)>>(iter: T) -> Self {
        let mut plan = Self::new();
        for (node, output) in iter {
            plan.subscribe(node, output);
        }
        plan
    }
}

#[cfg(test)]
mod output_subscription_tests {
    use super::*;

    #[test]
    fn subscriptions_are_deduplicated_without_reordering_endpoints() {
        let plan: OutputSubscriptionPlan = [(NodeId(2), 3), (NodeId(1), 4), (NodeId(2), 3)]
            .into_iter()
            .collect();

        assert_eq!(
            plan.outputs().collect::<Vec<_>>(),
            vec![(NodeId(2), 3), (NodeId(1), 4)]
        );
        assert_eq!(
            plan.retained_outputs().collect::<Vec<_>>(),
            vec![(NodeId(2), 3), (NodeId(1), 4)]
        );
    }

    #[test]
    fn retained_outputs_do_not_become_visible() {
        let mut plan = OutputSubscriptionPlan::new();
        plan.retain(NodeId(2), 3);

        assert!(!plan.contains(NodeId(2), 3));
        assert!(plan.is_retained(NodeId(2), 3));
    }

    #[test]
    fn sampling_overlay_subscriptions_are_deduplicated() {
        let mut plan = OutputSubscriptionPlan::new();
        plan.subscribe_sampling_overlay(NodeId(4));
        plan.subscribe_sampling_overlay(NodeId(4));

        assert!(plan.collects_sampling_overlay(NodeId(4)));
        assert_eq!(plan.sampling_overlays().collect::<Vec<_>>(), [NodeId(4)]);
    }
}
