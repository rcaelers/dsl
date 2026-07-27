use logic_analyzer_graph_api::node::GraphNodeRegistration;
use logic_analyzer_graph_api::node_support::PortKind;
use node_graph::api::{GraphDocumentBuilder, NodeTypeRegistry};

pub(crate) fn assert_node_registration_contract(stable_id: &str) {
    assert_node_registration_contract_with_state(stable_id, None);
}

pub(crate) fn assert_node_registration_contract_with_state(
    stable_id: &str,
    state: Option<serde_json::Value>,
) {
    let registration = inventory::iter::<GraphNodeRegistration>
        .into_iter()
        .find(|registration| registration.stable_id() == stable_id)
        .unwrap_or_else(|| panic!("missing graph-node registration '{stable_id}'"));

    let mut node_types = NodeTypeRegistry::new();
    registration.apply_node(&mut node_types);

    let mut document = GraphDocumentBuilder::new(node_types);
    let target = document
        .add_node(registration.name())
        .unwrap_or_else(|| panic!("isolated registry did not create '{}'", registration.name()));

    if let Some(state) = state {
        document.set_node_state(target, state);
    }

    let state = document.graph().nodes[&target].state.clone();
    let Some(builder) = registration.builder() else {
        return;
    };

    let target_inputs = document.graph().nodes[&target].inputs.clone();
    let target_outputs = document.graph().nodes[&target].outputs.clone();
    let mut required_inputs = Vec::new();
    for (index, socket) in target_inputs.iter().enumerate() {
        if !socket.visible || !builder.input_required(socket, &state) {
            continue;
        }
        let accepted = builder.accepted_kinds(socket, &state);
        assert!(
            !accepted.is_empty(),
            "{}.{} is required but accepts no runtime payload",
            registration.name(),
            socket.name
        );
        for kind in accepted {
            assert_port_mapping(
                builder.input_port(socket, 0, &state, kind),
                registration.name(),
                &socket.name,
                kind,
            );
        }
        required_inputs.push(index);
    }

    let mut offered_outputs = Vec::new();
    for (index, socket) in target_outputs.iter().enumerate() {
        if !socket.visible {
            continue;
        }
        let offered = builder.offered_kinds(socket, &state);
        assert!(
            !offered.is_empty(),
            "{}.{} is visible but offers no runtime payload",
            registration.name(),
            socket.name
        );
        for kind in offered {
            assert_port_mapping(
                builder.output_port(socket, &state, kind),
                registration.name(),
                &socket.name,
                kind,
            );
        }
        offered_outputs.push(index);
    }

    if builder.is_data_subscription() && required_inputs.is_empty() {
        let input = target_inputs
            .iter()
            .position(|socket| socket.visible)
            .expect("data subscription exposes an input");
        required_inputs.push(input);
    }

    let is_source = builder.is_source();
    let is_sink = builder.is_sink();
    let is_data_subscription = builder.is_data_subscription();

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
