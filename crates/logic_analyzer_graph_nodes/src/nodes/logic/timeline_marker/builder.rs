//! Runtime builders for timeline-marker nodes.

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, TimelineMarkerDescriptor, TimelineMarkerEdit,
    TimelineMarkerReference, TimelineMarkerReferenceBindingDescriptor,
    TimelineMarkerReferenceBindingEdit, TimelineMarkerReferenceChoice, parse_state,
};
use logic_analyzer_processing::nodes::logic::timeline_marker::{
    MarkerRelation as RuntimeMarkerRelation, TimelineMarkerRelation, TimelineMarkerSource,
    TimelineMarkerToTrigger, TimelineMarkerWindow,
};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, Sample, TimelineMarker, Trigger};

fn marker_kind() -> PortKind {
    PortKind::of_named::<TimelineMarker>("Timeline Marker")
}

pub(crate) fn register_timeline_marker_type() {
    signal_processing::register_type::<TimelineMarker>();
}

#[derive(Default)]
pub(crate) struct TimelineMarkerBuilder;

impl RuntimeBuilder for TimelineMarkerBuilder {
    fn is_source(&self) -> bool {
        true
    }

    fn is_time_domain_source(&self) -> bool {
        false
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![marker_kind()]
    }

    fn input_port(&self, _: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        None
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("marker".into())
    }

    fn timeline_markers(&self, state: &Value) -> Result<Vec<TimelineMarkerDescriptor>, String> {
        let state: super::definition::TimelineMarkerState = parse_state(state)?;
        Ok(vec![TimelineMarkerDescriptor::new(
            "marker",
            state.name.value,
            state.timestamp.value_ns,
        )])
    }

    fn apply_timeline_marker_edit(
        &self,
        state: &Value,
        edit: &TimelineMarkerEdit,
    ) -> Result<Option<Value>, String> {
        let mut state: super::definition::TimelineMarkerState = parse_state(state)?;
        match edit {
            TimelineMarkerEdit::SetTimestamp { id, timestamp_ns } if id == "marker" => {
                state.timestamp.value_ns = *timestamp_ns;
                serde_json::to_value(state)
                    .map(Some)
                    .map_err(|error| error.to_string())
            }
            TimelineMarkerEdit::SetTimestamp { id, .. } => {
                Err(format!("unknown timeline marker '{id}'"))
            }
        }
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::TimelineMarkerState = parse_state(state)?;
        Ok(Box::new(
            TimelineMarkerSource::new(state.timestamp.value_ns).with_name(name),
        ))
    }
}

#[derive(Default)]
pub(crate) struct CursorMarkerBuilder;

impl RuntimeBuilder for CursorMarkerBuilder {
    fn is_source(&self) -> bool {
        true
    }

    fn is_time_domain_source(&self) -> bool {
        false
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![marker_kind()]
    }

    fn input_port(&self, _: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        None
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("marker".into())
    }

    fn timeline_marker_reference_bindings(
        &self,
        state: &Value,
    ) -> Result<Vec<TimelineMarkerReferenceBindingDescriptor>, String> {
        let state: super::definition::CursorMarkerState = parse_state(state)?;
        Ok(vec![TimelineMarkerReferenceBindingDescriptor {
            id: "cursor".into(),
            selected: state
                .selected_cursor()
                .map(|number| TimelineMarkerReference::Cursor { number }),
            timestamp_ns: state.timestamp_ns(),
            choices: state
                .cursor_choices()
                .iter()
                .map(|choice| {
                    TimelineMarkerReferenceChoice::new(
                        TimelineMarkerReference::Cursor {
                            number: choice.number,
                        },
                        choice.label.clone(),
                        choice.timestamp_ns,
                    )
                })
                .collect(),
        }])
    }

    fn apply_timeline_marker_reference_binding_edit(
        &self,
        state: &Value,
        edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<Option<Value>, String> {
        let mut state: super::definition::CursorMarkerState = parse_state(state)?;
        let TimelineMarkerReferenceBindingEdit::Synchronize { id, choices } = edit;
        if id != "cursor" {
            return Err(format!("unknown timeline reference '{id}'"));
        }
        let choices = choices
            .iter()
            .map(|choice| match choice.reference {
                TimelineMarkerReference::Cursor { number } => {
                    super::definition::CursorChoiceValue {
                        number,
                        label: choice.label.clone(),
                        timestamp_ns: choice.timestamp_ns,
                    }
                }
            })
            .collect();
        state.synchronize_cursor_choices(choices);
        serde_json::to_value(state)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::CursorMarkerState = parse_state(state)?;
        let number = state.selected_cursor().ok_or_else(|| {
            "no cursor is available; add a cursor in the logic analyzer view".to_owned()
        })?;
        let marker = ctx
            .timeline_marker(TimelineMarkerReference::Cursor { number })
            .ok_or_else(|| {
                format!(
                    "cursor {number} is not available; add that cursor in the logic analyzer view"
                )
            })?;
        Ok(Box::new(
            TimelineMarkerSource::new(marker.timestamp_ns).with_name(name),
        ))
    }
}

#[derive(Default)]
pub(crate) struct MarkerToTriggerBuilder;

impl RuntimeBuilder for MarkerToTriggerBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![marker_kind()]
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Trigger>()]
    }

    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        Some("marker".into())
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("trigger".into())
    }

    fn build(
        &self,
        name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Ok(Box::new(TimelineMarkerToTrigger::new().with_name(name)))
    }
}

#[derive(Default)]
pub(crate) struct MarkerRelationBuilder;

impl RuntimeBuilder for MarkerRelationBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![marker_kind()]
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }

    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        Some("marker".into())
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("signal".into())
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::MarkerRelationState = parse_state(state)?;
        let relation = match state.relation.index {
            0 => RuntimeMarkerRelation::Before,
            _ => RuntimeMarkerRelation::AtOrAfter,
        };
        Ok(Box::new(
            TimelineMarkerRelation::new(relation).with_name(name),
        ))
    }
}

#[derive(Default)]
pub(crate) struct MarkerWindowBuilder;

impl RuntimeBuilder for MarkerWindowBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![marker_kind()]
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }

    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("start".into()),
            1 => Some("end".into()),
            _ => None,
        }
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("signal".into())
    }

    fn build(
        &self,
        name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Ok(Box::new(TimelineMarkerWindow::new().with_name(name)))
    }
}

#[cfg(test)]
mod builder_tests {
    use std::collections::HashMap;

    use signal_processing::{
        DerivedDataRetention, DerivedLanes, PersistentStoreConfig, SamplingActivity,
    };

    use super::*;

    #[derive(Default)]
    struct TestContext {
        lanes: DerivedLanes,
        markers: HashMap<TimelineMarkerReference, TimelineMarker>,
    }

    impl NodeBuildContext for TestContext {
        fn derived_lanes(&self) -> &DerivedLanes {
            &self.lanes
        }

        fn derived_data_retention(&self) -> DerivedDataRetention {
            DerivedDataRetention::Unlimited
        }

        fn derived_word_cache(&self, _member: usize) -> Option<&PersistentStoreConfig> {
            None
        }

        fn sampling_activity(
            &self,
            _runtime_name: &str,
            _input: usize,
        ) -> Option<SamplingActivity> {
            None
        }

        fn timeline_marker(&self, reference: TimelineMarkerReference) -> Option<TimelineMarker> {
            self.markers.get(&reference).copied()
        }
    }

    #[test]
    fn host_marker_edit_changes_only_the_owned_marker() {
        let state = serde_json::json!({
            "name": { "value": "Start" },
            "timestamp": { "value_ns": 10 }
        });
        let edited = TimelineMarkerBuilder
            .apply_timeline_marker_edit(
                &state,
                &TimelineMarkerEdit::SetTimestamp {
                    id: "marker".into(),
                    timestamp_ns: 25,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(edited["timestamp"]["value_ns"], 25);
        assert_eq!(edited["name"]["value"], "Start");
    }

    #[test]
    fn cursor_marker_resolves_the_selected_host_cursor() {
        let reference = TimelineMarkerReference::Cursor { number: 2 };
        let mut context = TestContext::default();
        context.markers.insert(reference, TimelineMarker::new(42));
        let state = serde_json::json!({
            "cursor": {
                "selected": 2,
                "choices": [
                    { "number": 2, "label": "Cursor 2", "timestamp_ns": 42 }
                ],
                "timestamp": { "value_ns": 42 }
            }
        });

        let process = CursorMarkerBuilder
            .build(
                "cursor-marker",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .unwrap();

        assert_eq!(process.name(), "cursor-marker");
    }

    #[test]
    fn cursor_choices_are_supplied_by_the_host_contract() {
        let state = serde_json::json!({
            "cursor": {
                "selected": 7,
                "choices": [],
                "timestamp": { "value_ns": 0 }
            }
        });
        let choices = vec![TimelineMarkerReferenceChoice::new(
            TimelineMarkerReference::Cursor { number: 3 },
            "Cursor 3",
            123,
        )];
        let edited = CursorMarkerBuilder
            .apply_timeline_marker_reference_binding_edit(
                &state,
                &TimelineMarkerReferenceBindingEdit::Synchronize {
                    id: "cursor".into(),
                    choices,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(edited["cursor"]["selected"], 3);
        assert_eq!(edited["cursor"]["timestamp"]["value_ns"], 123);
        assert_eq!(edited["cursor"]["choices"].as_array().unwrap().len(), 1);
    }
}
