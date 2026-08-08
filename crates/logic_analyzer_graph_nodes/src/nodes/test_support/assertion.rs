use std::collections::HashMap;

use logic_analyzer_graph_capabilities::node_support::{PortKind, ResolvedInput, ResolvedInputs};
use logic_analyzer_graph_editor_registry::graph_node_editor_registrations;
use logic_analyzer_graph_registry::{GraphRegistry, graph_node_registrations};
use node_graph::api::{GraphDocumentBuilder, NodeId, NodeTypeRegistry, Socket, SocketDirection};

use super::build_context::TestNodeBuildContext;

pub(crate) fn assert_node_registration_contract(stable_id: &str) {
    assert_node_registration_contract_impl(stable_id, None, true);
}

pub(crate) fn assert_node_registration_contract_with_state(
    stable_id: &str,
    state: Option<serde_json::Value>,
) {
    assert_node_registration_contract_impl(stable_id, state, true);
}

pub(crate) fn assert_node_registration_contract_without_runtime(stable_id: &str) {
    assert_node_registration_contract_impl(stable_id, None, false);
}

pub(crate) fn assert_node_registration_contract_without_runtime_with_state(
    stable_id: &str,
    state: serde_json::Value,
) {
    assert_node_registration_contract_impl(stable_id, Some(state), false);
}

fn assert_node_registration_contract_impl(
    stable_id: &str,
    state: Option<serde_json::Value>,
    build_runtime: bool,
) {
    let registration = graph_node_registrations()
        .into_iter()
        .find(|registration| registration.stable_id() == stable_id)
        .unwrap_or_else(|| panic!("missing graph-node registration '{stable_id}'"));

    let mut node_types = NodeTypeRegistry::new();
    graph_node_editor_registrations()
        .into_iter()
        .find(|editor| editor.stable_id() == stable_id)
        .unwrap_or_else(|| panic!("missing graph-node editor registration '{stable_id}'"))
        .apply_node(&mut node_types);

    let mut document = GraphDocumentBuilder::new(node_types);
    let target = document
        .add_node(registration.name())
        .unwrap_or_else(|| panic!("isolated registry did not create '{}'", registration.name()));

    if let Some(state) = state {
        document.set_node_state(target, state);
    }

    let state = document.graph().nodes[&target].state.clone();
    let registry =
        GraphRegistry::with_capability_overrides_and_infrastructure(Vec::new(), Vec::new());
    let Some(semantics) = registry.semantics(registration.name()) else {
        return;
    };

    let target_inputs = document.graph().nodes[&target].inputs.clone();
    let target_outputs = document.graph().nodes[&target].outputs.clone();
    let mut required_inputs = Vec::new();
    for (index, socket) in target_inputs.iter().enumerate() {
        let socket_reference = socket.reference(SocketDirection::Input, 0);
        if !socket.visible || !semantics.input_required(socket_reference, &state) {
            continue;
        }
        let accepted = semantics.accepted_kinds(socket_reference, &state);
        assert!(
            !accepted.is_empty(),
            "{}.{} is required but accepts no runtime payload",
            registration.name(),
            socket.name
        );
        for kind in accepted {
            assert_port_mapping(
                semantics.input_port(socket_reference, &state, kind),
                registration.name(),
                &socket.name,
                kind,
            );
        }
        required_inputs.push(index);
    }

    let mut offered_outputs = Vec::new();
    for (index, socket) in target_outputs.iter().enumerate() {
        let socket_reference = socket.reference(SocketDirection::Output, 0);
        if !socket.visible {
            continue;
        }
        let offered = semantics.offered_kinds(socket_reference, &state);
        assert!(
            !offered.is_empty(),
            "{}.{} is visible but offers no runtime payload",
            registration.name(),
            socket.name
        );
        for kind in offered {
            assert_port_mapping(
                semantics.output_port(socket_reference, &state, kind),
                registration.name(),
                &socket.name,
                kind,
            );
        }
        offered_outputs.push(index);
    }

    if semantics.is_data_subscription() && required_inputs.is_empty() {
        let input = target_inputs
            .iter()
            .position(|socket| socket.visible)
            .expect("data subscription exposes an input");
        required_inputs.push(input);
    }

    let is_source = semantics.is_source();
    let is_sink = semantics.is_sink();
    let is_data_subscription = semantics.is_data_subscription();

    if !is_source {
        assert!(
            !required_inputs.is_empty(),
            "non-source '{}' has no required runtime input",
            registration.name()
        );
    }

    if !is_sink && !is_data_subscription {
        assert!(
            !offered_outputs.is_empty(),
            "non-sink '{}' has no runtime output",
            registration.name()
        );
    }

    if build_runtime && !is_data_subscription {
        let resolved = resolved_inputs(semantics, &target_inputs, &state);
        let mut context = TestNodeBuildContext::default();
        let materializer = registry
            .materializer(registration.name())
            .unwrap_or_else(|| panic!("{} has semantics but no materializer", registration.name()));
        let runtime = materializer
            .build(registration.name(), &state, &resolved, &mut context)
            .unwrap_or_else(|error| {
                panic!(
                    "{} failed isolated runtime lowering: {error}",
                    registration.name()
                )
            });
        assert_eq!(runtime.name(), registration.name());

        if state.is_object() {
            let malformed = serde_json::Value::String("malformed fixture state".to_owned());
            let error = materializer
                .build(registration.name(), &malformed, &resolved, &mut context)
                .err()
                .unwrap_or_else(|| {
                    panic!(
                        "{} accepted malformed serialized state",
                        registration.name()
                    )
                });
            assert!(
                !error.to_string().trim().is_empty(),
                "{} returned an empty malformed-state error",
                registration.name()
            );
        }
    }
}

fn resolved_inputs(
    semantics: &dyn logic_analyzer_graph_capabilities::node::GraphNodeSemantics,
    sockets: &[Socket],
    state: &serde_json::Value,
) -> ResolvedInputs {
    let mut resolved = ResolvedInputs::default();
    let mut members = HashMap::<usize, usize>::new();

    for socket in sockets.iter().filter(|socket| socket.visible) {
        let member = members.entry(socket.def_index).or_default();
        let socket_reference = socket.reference(SocketDirection::Input, *member);
        let Some(kind) = semantics
            .accepted_kinds(socket_reference, state)
            .into_iter()
            .next()
        else {
            continue;
        };
        resolved.insert(
            socket.def_index,
            *member,
            ResolvedInput {
                kind,
                source: format!("fixture_{}_{}", socket.def_index, *member),
                source_node: NodeId(10_000 + socket.def_index as u32),
                source_output: socket.def_index,
                source_node_title: "Fixture source".to_owned(),
                source_output_title: format!("Output {}", socket.def_index),
                word_display_format: None,
                lane_presentation: None,
                default_lane_presentation: None,
                decoder_table_column: None,
                capture_channel: None,
            },
        );
        *member += 1;
    }

    resolved
}

fn assert_port_mapping(
    runtime_port: Option<String>,
    node_name: &str,
    socket_name: &str,
    kind: PortKind,
) {
    assert!(
        runtime_port.is_some(),
        "{node_name}.{socket_name} advertises {} without a runtime port mapping",
        kind.name()
    );
}
