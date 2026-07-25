use logic_analyzer_graph_api::node_support::ResolvedInput;
use node_graph::NodeId;

/// Application-supplied outputs whose produced data must be collected.
///
/// The compiler treats these endpoints as runtime subscriptions. It does not
/// interpret why an application subscribed or how the collected data is
/// presented.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputSubscriptionPlan {
    outputs: Vec<(NodeId, usize)>,
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
        if !self.contains(node, output) {
            self.outputs.push((node, output));
        }
    }

    pub fn contains(&self, node: NodeId, output: usize) -> bool {
        self.outputs.contains(&(node, output))
    }

    pub fn outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.outputs.iter().copied()
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
    }
}
