use serde::{Deserialize, Serialize};

use logic_analyzer_graph_api::node_support::ResolvedInput;
use node_graph::api::NodeId;

/// Application-supplied retained outputs and the subset currently presented.
///
/// Retention affects runtime collection. Visibility only selects metadata for
/// consumers of already-retained lanes; changing it does not alter execution.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSubscriptionPlan {
    visible_outputs: Vec<(NodeId, usize)>,
    retained_outputs: Vec<(NodeId, usize)>,
}

/// One retained lane produced for an application output subscription.
#[derive(Clone, Debug)]
pub struct CollectedOutputLane {
    /// Variadic input member that produced the retained lane.
    pub member: usize,
    /// Runtime lane name used by the derived-data store.
    pub lane_name: String,
    /// User-facing label for the source contributing the lane.
    pub source_label: String,
    /// Negotiated input and presentation metadata for the lane.
    pub input: ResolvedInput,
}

/// Runtime identities and source metadata for one collected output set.
#[derive(Clone, Debug)]
pub struct CollectedOutputSubscription {
    /// Runtime name of the collector node.
    pub runtime_name: String,
    /// Retained lanes produced by that collector.
    pub lanes: Vec<CollectedOutputLane>,
}

/// Retained lanes carrying decoder-table column metadata for one collector.
#[derive(Clone, Debug)]
pub struct CollectedTableSubscription {
    /// Graph node that owns the decoder-table collector.
    pub collector: NodeId,
    /// Retained lanes carrying decoder-table column descriptors.
    pub lanes: Vec<CollectedOutputLane>,
}

impl OutputSubscriptionPlan {
    /// Creates an empty output-subscription plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retains and makes one node output visible to application consumers.
    ///
    /// # Parameters
    /// - `node`: Source graph node.
    /// - `output`: Output-definition index on that node.
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

    /// Returns whether an endpoint is included in the visible-output subset.
    pub fn contains(&self, node: NodeId, output: usize) -> bool {
        self.visible_outputs.contains(&(node, output))
    }

    /// Returns whether retained.
    pub fn is_retained(&self, node: NodeId, output: usize) -> bool {
        self.retained_outputs.contains(&(node, output))
    }

    /// Iterates visible output endpoints in subscription order.
    pub fn outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.visible_outputs.iter().copied()
    }

    /// Iterates all retained endpoints in retention order.
    pub fn retained_outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.retained_outputs.iter().copied()
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
}
