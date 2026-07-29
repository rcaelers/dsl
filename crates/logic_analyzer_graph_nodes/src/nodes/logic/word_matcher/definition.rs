//! `Word Matcher` graph-node definition.

use egui::{Align, Color32, Layout, Rect, Ui, Vec2};
use serde::{Deserialize, Serialize};

use node_graph::{
    EnumValue, InlineControl, InputDef, IntValue, NodeBadge, NodeDef, NodeInstanceSchema,
    OutputDef, PanelSection, PropDef, Socket,
};

use crate::sockets::{COLOR_LOGIC, Signal, Trigger, Words};

const MATCH_OPS: &[&str] = &["==", "≠", "<", "≤", ">", "≥"];
const PREDICATES: &[&str] = &["Compare", "Inclusive range", "Set"];
const CURRENT_SCHEMA_VERSION: u32 = 2;
const MAX_MATCH_COUNT: i32 = 1_000_000;
const MAX_HOLDOFF_US: i32 = 2_000_000_000;

pub(crate) fn default_match_op() -> EnumValue {
    EnumValue::new(0, MATCH_OPS)
}

const TRIGGER_AT: &[&str] = &["Word start", "Word end"];

/// Default is "Word end": a command logically takes effect once it has
/// fully arrived (for instantaneous words the two coincide).
pub(crate) fn default_trigger_at() -> EnumValue {
    EnumValue::new(1, TRIGGER_AT)
}

fn default_predicate() -> EnumValue {
    EnumValue::new(0, PREDICATES)
}

fn default_range_min() -> MatcherTextValue {
    MatcherTextValue::new("0x00")
}

fn default_range_max() -> MatcherTextValue {
    MatcherTextValue::new("0xFF")
}

fn default_set_values() -> MatcherTextValue {
    MatcherTextValue::new("0x00, 0xFF")
}

fn default_match_count() -> IntValue {
    IntValue::new(1, 1, MAX_MATCH_COUNT)
}

fn default_holdoff_us() -> IntValue {
    IntValue::new(0, 0, MAX_HOLDOFF_US)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MatcherTextValue {
    pub(crate) value: String,
}

impl MatcherTextValue {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl InlineControl for MatcherTextValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
    ) -> bool {
        let previous = self.value.clone();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.style_mut().spacing.item_spacing = Vec2::splat(4.0 * zoom);
                ui.label(egui::RichText::new(label).size(10.0 * zoom));
                ui.add(
                    egui::TextEdit::singleline(&mut self.value)
                        .desired_width(ui.available_width().max(24.0 * zoom)),
                );
            },
        );
        self.value != previous
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WordMatcherState {
    #[serde(default)]
    pub(crate) schema_version: u32,
    pub(crate) pattern: MatcherTextValue,
    pub(crate) mask: MatcherTextValue,
    #[serde(default = "default_predicate")]
    pub(crate) predicate: EnumValue,
    /// Comparison of the masked word against the masked pattern.
    #[serde(default = "default_match_op")]
    pub(crate) op: EnumValue,
    /// Whether the trigger lands at the matched word's start or end.
    #[serde(default = "default_trigger_at")]
    pub(crate) trigger_at: EnumValue,
    #[serde(default = "default_range_min")]
    pub(crate) range_min: MatcherTextValue,
    #[serde(default = "default_range_max")]
    pub(crate) range_max: MatcherTextValue,
    #[serde(default = "default_set_values")]
    pub(crate) set_values: MatcherTextValue,
    #[serde(default = "default_match_count")]
    pub(crate) match_count: IntValue,
    #[serde(default = "default_holdoff_us")]
    pub(crate) holdoff_us: IntValue,
    #[serde(skip)]
    pub(crate) compatibility_warning: Option<String>,
}

pub(crate) struct WordMatcher;
impl NodeDef for WordMatcher {
    type State = WordMatcherState;

    fn name() -> &'static str {
        "Word Matcher"
    }
    fn category() -> &'static str {
        "Logic"
    }
    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Words>("Words").stable_id("words"),
            InputDef::new::<Trigger>("Rearm").stable_id("rearm"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![
            OutputDef::new::<Trigger>("Match").stable_id("match"),
            OutputDef::new::<Signal>("Matched").stable_id("matched"),
            OutputDef::new::<Words>("Matching Words").stable_id("matching_words"),
        ]
    }

    fn state() -> Self::State {
        WordMatcherState {
            schema_version: CURRENT_SCHEMA_VERSION,
            pattern: MatcherTextValue::new("0x000000"),
            mask: MatcherTextValue::new("0xFFFFFF"),
            predicate: default_predicate(),
            op: default_match_op(),
            trigger_at: default_trigger_at(),
            range_min: default_range_min(),
            range_max: default_range_max(),
            set_values: default_set_values(),
            match_count: default_match_count(),
            holdoff_us: default_holdoff_us(),
            compatibility_warning: None,
        }
    }

    fn props() -> Vec<PropDef<Self::State>> {
        vec![PropDef::control("pattern", "Pattern", |state| {
            &mut state.pattern
        })]
    }

    fn instance_schema(state: &Self::State) -> NodeInstanceSchema<Self::State> {
        let props = if state.predicate.selected() == "Compare" {
            Self::props()
        } else {
            Vec::new()
        };
        NodeInstanceSchema::new(Self::inputs(), Self::outputs())
            .props(props)
            .panel(predicate_panel(state))
            .panels(Self::panels())
    }

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        predicate_panel(&Self::state())
    }

    fn migrate_saved_sockets(
        state: &mut Self::State,
        inputs: &mut Vec<Socket>,
        outputs: &mut Vec<Socket>,
    ) {
        let migrated = migrate_socket_identity(inputs, "Words", "words")
            | migrate_socket_identity(outputs, "Match", "match")
            | migrate_socket_identity(outputs, "Matched", "matched")
            | migrate_socket_identity(outputs, "Matching Words", "matching_words");
        if migrated {
            state.compatibility_warning = Some(
                "Updated legacy Word Matcher socket identities; existing connections were preserved"
                    .to_owned(),
            );
        }
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        if state.schema_version < CURRENT_SCHEMA_VERSION {
            let sockets_migrated = state.compatibility_warning.is_some();
            state.schema_version = CURRENT_SCHEMA_VERSION;
            state.compatibility_warning = Some(if sockets_migrated {
                "Upgraded the saved Word Matcher with predicate, rearm, matching-word controls, and current socket identities; existing connections were preserved"
                    .to_owned()
            } else {
                "Upgraded the saved Word Matcher with predicate, rearm, and matching-word controls"
                    .to_owned()
            });
        }
        state.match_count.min = 1;
        state.match_count.max = MAX_MATCH_COUNT;
        state.match_count.value = state.match_count.value.clamp(1, MAX_MATCH_COUNT);
        state.holdoff_us.min = 0;
        state.holdoff_us.max = MAX_HOLDOFF_US;
        state.holdoff_us.value = state.holdoff_us.value.clamp(0, MAX_HOLDOFF_US);
    }

    fn badge(state: &Self::State) -> Option<NodeBadge> {
        if super::super::word_value::parse_hex_u64(&state.mask.value).is_err() {
            return Some(NodeBadge::error("Invalid hex mask"));
        }
        let predicate_error = match state.predicate.selected() {
            "Inclusive range" => {
                let minimum = super::super::word_value::parse_hex_u64(&state.range_min.value);
                let maximum = super::super::word_value::parse_hex_u64(&state.range_max.value);
                match (minimum, maximum) {
                    (Ok(minimum), Ok(maximum)) if minimum <= maximum => None,
                    (Ok(_), Ok(_)) => Some("Range minimum exceeds maximum"),
                    _ => Some("Invalid hexadecimal range"),
                }
            }
            "Set" => super::super::word_value::parse_hex_set(&state.set_values.value)
                .is_err()
                .then_some("Invalid hexadecimal value set"),
            _ => super::super::word_value::parse_hex_u64(&state.pattern.value)
                .is_err()
                .then_some("Invalid hex pattern"),
        };
        predicate_error
            .map(NodeBadge::error)
            .or_else(|| state.compatibility_warning.as_ref().map(NodeBadge::warning))
    }
}

fn migrate_socket_identity(sockets: &mut Vec<Socket>, legacy: &str, current: &str) -> bool {
    let Some(mut legacy_index) = sockets.iter().position(|socket| socket.schema_id == legacy)
    else {
        return false;
    };
    if let Some(current_index) = sockets
        .iter()
        .position(|socket| socket.schema_id == current)
    {
        sockets.remove(current_index);
        if current_index < legacy_index {
            legacy_index -= 1;
        }
    }
    sockets[legacy_index].schema_id = current.to_owned();
    true
}

fn predicate_panel(state: &WordMatcherState) -> Vec<PanelSection<WordMatcherState>> {
    let mut props: Vec<PropDef<WordMatcherState>> =
        vec![PropDef::control("predicate", "Predicate", |state| {
            &mut state.predicate
        })];
    match state.predicate.selected() {
        "Inclusive range" => {
            props.push(PropDef::control("range_min", "Minimum", |state| {
                &mut state.range_min
            }));
            props.push(PropDef::control("range_max", "Maximum", |state| {
                &mut state.range_max
            }));
            props.push(PropDef::control("mask", "Mask", |state| &mut state.mask));
        }
        "Set" => {
            props.push(PropDef::control("set_values", "Values", |state| {
                &mut state.set_values
            }));
            props.push(PropDef::control("mask", "Mask", |state| &mut state.mask));
        }
        _ => {
            props.push(PropDef::control("op", "Compare", |state| &mut state.op));
            props.push(PropDef::control("pattern", "Pattern", |state| {
                &mut state.pattern
            }));
            props.push(PropDef::control("mask", "Mask", |state| &mut state.mask));
        }
    }
    props.push(PropDef::control("match_count", "Emit every", |state| {
        &mut state.match_count
    }));
    props.push(PropDef::control("holdoff_us", "Holdoff µs", |state| {
        &mut state.holdoff_us
    }));
    props.push(PropDef::control("trigger_at", "Trigger at", |state| {
        &mut state.trigger_at
    }));
    vec![PanelSection::new("Options", props)]
}

#[cfg(test)]
mod definition_tests {
    use node_graph::NodeDef;

    use super::*;

    #[test]
    fn legacy_comparison_state_migrates_with_a_visible_warning() {
        let legacy = serde_json::json!({
            "pattern": { "value": "0xAA" },
            "mask": { "value": "0xFF" },
            "op": { "index": 0, "variants": MATCH_OPS },
            "trigger_at": { "index": 1, "variants": TRIGGER_AT }
        });
        let mut state: WordMatcherState = serde_json::from_value(legacy).unwrap();
        WordMatcher::on_update(&mut state, &mut [], &mut []);

        assert_eq!(state.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(state.predicate.selected(), "Compare");
        assert_eq!(state.match_count.value, 1);
        assert!(WordMatcher::badge(&state).is_some_and(|badge| badge.text.contains("Upgraded")));
    }

    #[test]
    fn legacy_socket_identities_migrate_before_reconciliation() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(WordMatcher::name(), egui::Pos2::ZERO)
            .unwrap();
        let mut graph = widget.graph().clone();
        let saved = graph.nodes.get_mut(&node).unwrap();
        let current_words_input = saved.inputs[0].clone();
        saved.inputs[0].schema_id = "Words".to_owned();
        saved.inputs.push(current_words_input);
        let current_outputs = saved.outputs.clone();
        saved.outputs[0].schema_id = "Match".to_owned();
        saved.outputs[1].schema_id = "Matched".to_owned();
        saved.outputs[2].schema_id = "Matching Words".to_owned();
        saved.outputs.extend(current_outputs);
        saved.state["schema_version"] = serde_json::json!(0);

        widget.set_graph(graph);

        let restored = &widget.graph().nodes[&node];
        assert_eq!(
            restored
                .inputs
                .iter()
                .map(|socket| socket.schema_id.as_str())
                .collect::<Vec<_>>(),
            ["words", "rearm"]
        );
        assert_eq!(
            restored
                .outputs
                .iter()
                .map(|socket| socket.schema_id.as_str())
                .collect::<Vec<_>>(),
            ["match", "matched", "matching_words"]
        );
        assert!(
            restored
                .badge
                .as_ref()
                .is_some_and(|badge| badge.text.contains("existing connections were preserved"))
        );
    }

    #[test]
    fn predicate_validation_only_checks_the_selected_operand_family() {
        let mut state = WordMatcher::state();
        state.pattern.value = "not hex".to_owned();
        state.predicate.index = 1;
        state.range_min.value = "0x10".to_owned();
        state.range_max.value = "0x20".to_owned();
        assert!(WordMatcher::badge(&state).is_none());

        state.range_min.value = "0x30".to_owned();
        assert!(WordMatcher::badge(&state).is_some_and(|badge| badge.text.contains("exceeds")));
    }

    #[test]
    fn instance_schema_contains_only_controls_for_the_selected_predicate() {
        let mut state = WordMatcher::state();
        let compare = WordMatcher::instance_schema(&state);
        assert_eq!(compare.props.len(), 1);
        assert_eq!(compare.panel[0].props.len(), 7);

        state.predicate.select("Inclusive range");
        let range = WordMatcher::instance_schema(&state);
        assert!(range.props.is_empty());
        assert_eq!(range.panel[0].props.len(), 7);

        state.predicate.select("Set");
        let set = WordMatcher::instance_schema(&state);
        assert!(set.props.is_empty());
        assert_eq!(set.panel[0].props.len(), 6);
    }
}
