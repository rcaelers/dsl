//! Graph-node definitions for timeline markers.

use egui::{Align, Color32, Layout, Rect, Ui};
use serde::{Deserialize, Deserializer, Serialize};

use node_graph::{
    EnumValue, InlineControl, InputDef, NodeBadge, NodeDef, OutputDef, PanelSection, PropDef,
    StringValue,
};

use crate::sockets::{COLOR_LOGIC, Signal, TimelineMarker as TimelineMarkerSocket, Trigger};

const RELATIONS: &[&str] = &["Before marker", "At or after marker"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MarkerTimeValue {
    pub(crate) value_ns: u64,
}

impl MarkerTimeValue {
    fn new(value_ns: u64) -> Self {
        Self { value_ns }
    }
}

impl Default for MarkerTimeValue {
    fn default() -> Self {
        Self::new(250_000_000)
    }
}

impl InlineControl for MarkerTimeValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
    ) -> bool {
        let old = self.value_ns;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.label(egui::RichText::new(label).size(10.0 * zoom));
                ui.add(
                    egui::DragValue::new(&mut self.value_ns)
                        .speed(1_000.0)
                        .suffix(" ns"),
                );
            },
        );
        self.value_ns != old
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TimelineMarkerState {
    pub(crate) name: StringValue,
    pub(crate) timestamp: MarkerTimeValue,
}

pub(crate) struct TimelineMarker;

impl NodeDef for TimelineMarker {
    type State = TimelineMarkerState;

    fn name() -> &'static str {
        "Timeline Marker"
    }

    fn category() -> &'static str {
        "Timeline::Markers"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        Vec::new()
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<TimelineMarkerSocket>("Marker").stable_id("marker")]
    }

    fn state() -> Self::State {
        TimelineMarkerState {
            name: StringValue::new("Timeline Marker"),
            timestamp: MarkerTimeValue::new(250_000_000),
        }
    }

    fn title(state: &mut Self::State) -> Option<&mut StringValue> {
        Some(&mut state.name)
    }

    fn props() -> Vec<PropDef<Self::State>> {
        vec![PropDef::control("name", "Name", |state| &mut state.name)]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Marker",
            vec![PropDef::control("timestamp", "Time", |state| {
                &mut state.timestamp
            })],
        )]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CursorChoiceValue {
    pub(crate) number: u32,
    pub(crate) label: String,
    pub(crate) timestamp_ns: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CursorSelectionValue {
    pub(crate) selected: Option<u32>,
    pub(crate) choices: Vec<CursorChoiceValue>,
    pub(crate) timestamp: MarkerTimeValue,
    #[serde(skip)]
    migrated_from_numeric: bool,
}

impl<'de> Deserialize<'de> for CursorSelectionValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SavedValue {
            #[serde(default)]
            selected: Option<u32>,
            #[serde(default)]
            value: Option<i32>,
            #[serde(default)]
            choices: Vec<CursorChoiceValue>,
            #[serde(default)]
            timestamp: MarkerTimeValue,
        }

        let saved = SavedValue::deserialize(deserializer)?;
        let migrated_from_numeric = saved.value.is_some();
        let selected = saved.selected.or_else(|| {
            saved
                .value
                .and_then(|value| u32::try_from(value.max(1)).ok())
        });
        Ok(Self {
            selected,
            choices: saved.choices,
            timestamp: saved.timestamp,
            migrated_from_numeric,
        })
    }
}

impl CursorSelectionValue {
    fn selected_label(&self) -> &str {
        self.selected
            .and_then(|selected| self.choices.iter().find(|choice| choice.number == selected))
            .map(|choice| choice.label.as_str())
            .unwrap_or_else(|| {
                if self.choices.is_empty() {
                    "No cursors available"
                } else {
                    "Choose cursor"
                }
            })
    }
}

impl InlineControl for CursorSelectionValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
    ) -> bool {
        let old = self.selected;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.label(egui::RichText::new(label).size(10.0 * zoom));
                let selected_text = self.selected_label().to_owned();
                egui::ComboBox::from_id_salt("cursor-marker-choice")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for choice in &self.choices {
                            ui.selectable_value(
                                &mut self.selected,
                                Some(choice.number),
                                &choice.label,
                            );
                        }
                    });
            },
        );
        if self.selected != old
            && let Some(choice) = self
                .selected
                .and_then(|selected| self.choices.iter().find(|choice| choice.number == selected))
        {
            self.timestamp.value_ns = choice.timestamp_ns;
        }
        self.selected != old
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CursorMarkerState {
    pub(crate) cursor: CursorSelectionValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compatibility_warning: Option<String>,
}

pub(crate) struct CursorMarker;

impl NodeDef for CursorMarker {
    type State = CursorMarkerState;

    fn name() -> &'static str {
        "Cursor Marker"
    }

    fn category() -> &'static str {
        "Timeline::Markers"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        Vec::new()
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<TimelineMarkerSocket>("Marker").stable_id("marker")]
    }

    fn state() -> Self::State {
        CursorMarkerState {
            cursor: CursorSelectionValue {
                selected: Some(1),
                choices: Vec::new(),
                timestamp: MarkerTimeValue::default(),
                migrated_from_numeric: false,
            },
            compatibility_warning: None,
        }
    }

    fn on_update(
        state: &mut Self::State,
        _inputs: &mut [node_graph::Socket],
        _outputs: &mut [node_graph::Socket],
    ) {
        if state.cursor.migrated_from_numeric {
            state.compatibility_warning = Some(
                "Migrated the legacy numeric cursor setting; available cursors now come from the logic analyzer view"
                    .into(),
            );
            state.cursor.migrated_from_numeric = false;
        }
    }

    fn badge(state: &Self::State) -> Option<NodeBadge> {
        state.compatibility_warning.as_ref().map(NodeBadge::warning)
    }

    fn props() -> Vec<PropDef<Self::State>> {
        vec![PropDef::control("cursor", "Cursor", |state| {
            &mut state.cursor
        })]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Cursor",
            vec![
                PropDef::control("cursor", "Cursor", |state| &mut state.cursor),
                PropDef::control("timestamp", "Time", |state| &mut state.cursor.timestamp),
            ],
        )]
    }
}

pub(crate) struct MarkerToTrigger;

impl NodeDef for MarkerToTrigger {
    type State = ();

    fn name() -> &'static str {
        "Marker to Trigger"
    }

    fn category() -> &'static str {
        "Timeline::Support"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<TimelineMarkerSocket>("Marker").stable_id("marker")]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Trigger>("Trigger").stable_id("trigger")]
    }

    fn state() -> Self::State {}

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MarkerRelationState {
    pub(crate) relation: EnumValue,
}

pub(crate) struct MarkerRelation;

impl NodeDef for MarkerRelation {
    type State = MarkerRelationState;

    fn name() -> &'static str {
        "Marker Relation"
    }

    fn category() -> &'static str {
        "Timeline::Support"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<TimelineMarkerSocket>("Marker").stable_id("marker")]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Signal>("Signal").stable_id("signal")]
    }

    fn state() -> Self::State {
        MarkerRelationState {
            relation: EnumValue::new(1, RELATIONS),
        }
    }

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Relation",
            vec![PropDef::control("relation", "High", |state| {
                &mut state.relation
            })],
        )]
    }
}

pub(crate) struct MarkerWindow;

impl NodeDef for MarkerWindow {
    type State = ();

    fn name() -> &'static str {
        "Marker Window"
    }

    fn category() -> &'static str {
        "Timeline::Support"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<TimelineMarkerSocket>("Start").stable_id("start"),
            InputDef::new::<TimelineMarkerSocket>("End").stable_id("end"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Signal>("Inside [start, end)").stable_id("signal")]
    }

    fn state() -> Self::State {}

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }
}

#[cfg(test)]
mod definition_tests {
    use node_graph::NodeDef;

    use super::*;

    #[test]
    fn timeline_nodes_expose_explicit_marker_conversion_sockets() {
        assert_eq!(TimelineMarker::category(), "Timeline::Markers");
        assert_eq!(CursorMarker::category(), "Timeline::Markers");
        assert_eq!(MarkerToTrigger::category(), "Timeline::Support");
        assert_eq!(MarkerRelation::category(), "Timeline::Support");
        assert_eq!(MarkerWindow::category(), "Timeline::Support");
        assert_eq!(TimelineMarker::outputs().len(), 1);
        assert_eq!(TimelineMarker::props().len(), 1);
        assert_eq!(CursorMarker::outputs().len(), 1);
        assert_eq!(CursorMarker::props().len(), 1);
        assert_eq!(MarkerToTrigger::inputs().len(), 1);
        assert_eq!(MarkerWindow::inputs().len(), 2);
    }

    #[test]
    fn legacy_numeric_cursor_state_loads_as_a_selection() {
        let state: CursorMarkerState = serde_json::from_value(serde_json::json!({
            "cursor": { "value": 2, "min": 1, "max": 2147483647 }
        }))
        .unwrap();

        assert_eq!(state.cursor.selected, Some(2));
        assert!(state.cursor.choices.is_empty());
        assert!(state.cursor.migrated_from_numeric);
    }
}
