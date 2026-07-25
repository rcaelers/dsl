//! Runtime builder for `Viewer`.

use std::collections::HashMap;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{PortKind, ResolvedInputs, parse_state};
use node_graph::Socket;

#[derive(Default)]
pub(crate) struct ViewerSubscriptionBuilder;

impl RuntimeBuilder for ViewerSubscriptionBuilder {
    fn is_data_subscription(&self) -> bool {
        true
    }
    fn collected_lane_names(
        &self,
        state: &Value,
        resolved: &ResolvedInputs,
    ) -> Vec<(usize, String)> {
        let Ok(state) = parse_state::<super::definition::ViewerState>(state) else {
            return Vec::new();
        };
        let prefix = state.label.value.trim();
        let mut counts: HashMap<String, usize> = HashMap::new();
        resolved
            .members(0)
            .into_iter()
            .map(|(member, input)| {
                let base = if prefix.is_empty() {
                    input.source.clone()
                } else {
                    format!("{prefix}: {}", input.source)
                };
                let count = counts.entry(base.clone()).or_default();
                *count += 1;
                let name = if *count == 1 {
                    base
                } else {
                    format!("{base} ({count})")
                };
                (member, name)
            })
            .collect()
    }
    fn collected_source_label(&self, state: &Value, source_title: &str) -> String {
        let Ok(state) = parse_state::<super::definition::ViewerState>(state) else {
            return source_title.to_owned();
        };
        let prefix = state.label.value.trim();
        if prefix.is_empty() {
            source_title.to_owned()
        } else {
            format!("{prefix}: {source_title}")
        }
    }
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        // `lower()` supplies the registry's subscribed payload kinds for a
        // data subscription. Keeping this empty prevents a second, fixed
        // built-in list from becoming the source of truth.
        Vec::new()
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn input_port(
        &self,
        _socket: &Socket,
        member_index: usize,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some(format!("in{member_index}"))
    }
    fn output_port(&self, _: &Socket, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn input_required(&self, _: &Socket, _: &Value) -> bool {
        // A lane-less viewer is pointless but harmless.
        false
    }
}
