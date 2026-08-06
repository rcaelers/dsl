//! Presentation-neutral collection of retained derived data.

use std::collections::HashMap;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{NodeBuildContext, PortKind, ResolvedInputs};
use node_graph_document::SocketReference;
use signal_runtime::ProcessNode;

pub(crate) const BUILDER_NAME: &str = "Derived Data Collector";
pub(crate) const OUTPUT_SUBSCRIPTION_BUILDER_NAME: &str = "Output Subscription Collector";

pub(crate) struct DataCollectorBuilder {
    output_subscription: bool,
}

impl DataCollectorBuilder {
    pub(crate) const fn retained_data() -> Self {
        Self {
            output_subscription: false,
        }
    }

    pub(crate) const fn output_subscription() -> Self {
        Self {
            output_subscription: true,
        }
    }

    fn default_lane_names(resolved: &ResolvedInputs) -> Vec<(usize, String)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        resolved
            .members(0)
            .into_iter()
            .map(|(member, input)| {
                let count = counts.entry(input.source.clone()).or_default();
                *count += 1;
                let name = if *count == 1 {
                    input.source.clone()
                } else {
                    format!("{} ({count})", input.source)
                };
                (member, name)
            })
            .collect()
    }
}

impl GraphNodeSemantics for DataCollectorBuilder {
    fn is_sink(&self) -> bool {
        true
    }

    fn is_data_collector(&self) -> bool {
        true
    }

    fn is_data_subscription(&self) -> bool {
        self.output_subscription
    }

    fn collected_lane_names(
        &self,
        _state: &Value,
        resolved: &ResolvedInputs,
    ) -> Vec<(usize, String)> {
        Self::default_lane_names(resolved)
    }

    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn input_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some(format!("in{}", socket.member_index()))
    }

    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        None
    }

    fn input_required(&self, _socket: SocketReference<'_>, _state: &Value) -> bool {
        false
    }
}

impl RuntimeMaterializer for DataCollectorBuilder {
    fn build(
        &self,
        _name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Err("data collectors must be materialized through the payload registry".to_owned())
    }
}
