//! Compiler-owned assembly of graph-node inventory submissions.

use std::collections::HashMap;

use logic_analyzer_graph_api::node::{
    GraphNodeRegistration, RuntimeBuilder, RuntimeBuilderOverride, graph_node_registrations,
};

pub(crate) fn standard_graph_node_builders(
    overrides: Vec<RuntimeBuilderOverride>,
) -> HashMap<String, Box<dyn RuntimeBuilder>> {
    let mut builders: HashMap<String, Box<dyn RuntimeBuilder>> = HashMap::new();
    let mut overrides = overrides
        .into_iter()
        .map(|override_builder| {
            let stable_id = override_builder.stable_id().to_owned();
            (stable_id, override_builder.into_builder())
        })
        .collect::<HashMap<_, _>>();
    builders.insert(
        super::DATA_COLLECTOR_BUILDER.into(),
        Box::new(super::DataCollectorBuilder::retained_data()),
    );
    builders.insert(
        super::OUTPUT_SUBSCRIPTION_BUILDER_NAME.into(),
        Box::new(super::DataCollectorBuilder::output_subscription()),
    );
    for registration in graph_node_registrations() {
        registration.apply_runtime_setup();
        let builder = overrides
            .remove(registration.stable_id())
            .or_else(|| registration.builder());
        let Some(builder) = builder else {
            continue;
        };
        assert!(
            builders
                .insert(registration.name().to_owned(), builder)
                .is_none(),
            "graph-node inventory builder '{}' conflicts with an explicit catalog entry",
            registration.name()
        );
    }
    assert!(
        overrides.is_empty(),
        "host runtime-builder override targets unregistered node(s): {}",
        overrides.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    builders
}

pub(crate) fn validate_graph_node_payload_requirements(
    payloads: &signal_processing::PayloadRegistry,
) {
    validate_graph_node_payload_requirements_for(&graph_node_registrations(), payloads);
}

fn validate_graph_node_payload_requirements_for(
    registrations: &[&GraphNodeRegistration],
    payloads: &signal_processing::PayloadRegistry,
) {
    for registration in registrations {
        for stable_id in registration.required_payloads() {
            assert!(
                payloads.descriptor_by_stable_id(stable_id).is_some(),
                "graph-node inventory feature '{}' requires unavailable payload '{}'",
                registration.stable_id(),
                stable_id
            );
        }
    }
}
