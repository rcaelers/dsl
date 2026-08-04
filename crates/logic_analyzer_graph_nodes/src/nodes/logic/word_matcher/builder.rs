//! Runtime builder for `Word Matcher` — fires a trigger when a decoded word matches a
//! pattern/mask. Works on any decoder's `Word` output, no decoder-specific
//! knowledge needed (kind negotiation, `docs/APP_DESIGN.md`).

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::RuntimeBuilder;
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::logic::word_matcher::{
    MatchOp, PredicateMode, TriggerAt, WordMatcher,
};
use node_graph::api::Socket;
use signal_processing::{ConfigValue, NodeConfig, ProcessNode, Sample, Trigger, Word};

#[derive(Default)]
pub(crate) struct WordMatcherBuilder;

impl WordMatcherBuilder {
    /// UI op glyph → runtime `MatchOp` and its config wire name.
    fn match_op(selected: &str) -> (MatchOp, &'static str) {
        match selected {
            "≠" => (MatchOp::Ne, "ne"),
            "<" => (MatchOp::Lt, "lt"),
            "≤" => (MatchOp::Le, "le"),
            ">" => (MatchOp::Gt, "gt"),
            "≥" => (MatchOp::Ge, "ge"),
            _ => (MatchOp::Eq, "eq"),
        }
    }

    /// UI "Trigger at" selection → runtime `TriggerAt` and its wire name.
    fn trigger_at(selected: &str) -> (TriggerAt, &'static str) {
        match selected {
            "Word start" => (TriggerAt::Start, "start"),
            _ => (TriggerAt::End, "end"),
        }
    }

    fn predicate(selected: &str) -> (PredicateMode, &'static str) {
        match selected {
            "Inclusive range" => (PredicateMode::InclusiveRange, "range"),
            "Set" => (PredicateMode::Set, "set"),
            _ => (PredicateMode::Compare, "compare"),
        }
    }
}

impl RuntimeBuilder for WordMatcherBuilder {
    fn accepted_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        match socket.def_index {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of::<Trigger>()],
            _ => vec![],
        }
    }
    fn offered_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        match socket.def_index {
            0 => vec![PortKind::of::<Trigger>()],
            1 => vec![PortKind::of::<Sample>()],
            2 => vec![PortKind::of::<Word>()],
            _ => vec![],
        }
    }
    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("words".into()),
            1 => Some("rearm".into()),
            _ => None,
        }
    }
    fn output_port(&self, socket: &Socket, _state: &Value, _kind: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("trigger".into()),
            1 => Some("matched".into()),
            2 => Some("matching_words".into()),
            _ => None,
        }
    }
    fn input_required(&self, socket: &Socket, _state: &Value) -> bool {
        socket.def_index == 0
    }
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::WordMatcherState = parse_state(state)?;
        let mask = super::super::word_value::parse_hex_u64(&state.mask.value)?;
        let (op, _) = Self::match_op(state.op.selected());
        let (trigger_at, _) = Self::trigger_at(state.trigger_at.selected());
        let (predicate, _) = Self::predicate(state.predicate.selected());
        let pattern = if predicate == PredicateMode::Compare {
            super::super::word_value::parse_hex_u64(&state.pattern.value)?
        } else {
            0
        };
        let mut matcher = WordMatcher::new(pattern, mask)
            .with_op(op)
            .with_trigger_at(trigger_at)
            .with_match_count(state.match_count.value.max(1) as u64)
            .with_holdoff_ns((state.holdoff_us.value.max(0) as u64).saturating_mul(1_000))
            .with_manual_rearm(resolved.kind(1).is_some())
            .with_name(name);
        matcher = match predicate {
            PredicateMode::Compare => matcher,
            PredicateMode::InclusiveRange => matcher.with_inclusive_range(
                super::super::word_value::parse_hex_u64(&state.range_min.value)?,
                super::super::word_value::parse_hex_u64(&state.range_max.value)?,
            ),
            PredicateMode::Set => matcher.with_set(super::super::word_value::parse_hex_set(
                &state.set_values.value,
            )?),
        };
        Ok(Box::new(matcher))
    }

    fn hot_config(&self, state: &Value) -> Option<NodeConfig> {
        let state: super::definition::WordMatcherState = parse_state(state).ok()?;
        let mut config = NodeConfig::new();
        config.insert(
            "mask".into(),
            ConfigValue::U64(super::super::word_value::parse_hex_u64(&state.mask.value).ok()?),
        );
        let (_, op_name) = Self::match_op(state.op.selected());
        config.insert("op".into(), ConfigValue::Text(op_name.into()));
        let (_, trigger_at_name) = Self::trigger_at(state.trigger_at.selected());
        config.insert(
            "trigger_at".into(),
            ConfigValue::Text(trigger_at_name.into()),
        );
        let (predicate, predicate_name) = Self::predicate(state.predicate.selected());
        config.insert("predicate".into(), ConfigValue::Text(predicate_name.into()));
        match predicate {
            PredicateMode::Compare => {
                config.insert(
                    "pattern".into(),
                    ConfigValue::U64(
                        super::super::word_value::parse_hex_u64(&state.pattern.value).ok()?,
                    ),
                );
            }
            PredicateMode::InclusiveRange => {
                config.insert(
                    "range_min".into(),
                    ConfigValue::U64(
                        super::super::word_value::parse_hex_u64(&state.range_min.value).ok()?,
                    ),
                );
                config.insert(
                    "range_max".into(),
                    ConfigValue::U64(
                        super::super::word_value::parse_hex_u64(&state.range_max.value).ok()?,
                    ),
                );
            }
            PredicateMode::Set => {
                let set = super::super::word_value::parse_hex_set(&state.set_values.value).ok()?;
                config.insert(
                    "set".into(),
                    ConfigValue::Text(
                        set.into_iter()
                            .map(|value| format!("0x{value:X}"))
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                );
            }
        }
        config.insert(
            "match_count".into(),
            ConfigValue::U64(state.match_count.value.max(1) as u64),
        );
        config.insert(
            "holdoff_ns".into(),
            ConfigValue::U64((state.holdoff_us.value.max(0) as u64).saturating_mul(1_000)),
        );
        Some(config)
    }
}

#[cfg(test)]
mod builder_tests {
    use node_graph::NodeDef;

    use super::super::definition::WordMatcher;
    use super::*;

    #[test]
    fn words_are_required_but_rearm_is_optional_and_matching_words_are_offered() {
        let builder = WordMatcherBuilder;
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node_id = widget
            .add_node_at(WordMatcher::name(), egui::Pos2::ZERO)
            .unwrap();
        let node = &widget.graph().nodes[&node_id];

        assert!(builder.input_required(&node.inputs[0], &node.state));
        assert!(!builder.input_required(&node.inputs[1], &node.state));
        assert_eq!(
            builder.offered_kinds(&node.outputs[2], &node.state),
            [PortKind::of::<Word>()]
        );
    }
}
