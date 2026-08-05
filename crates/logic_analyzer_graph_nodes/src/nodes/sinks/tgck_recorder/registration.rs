inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::TgckRecorder,
        super::builder::TgckRecorderBuilder,
        super::builder::TgckRecorderBuilder,
    >("org.logicconduit.graph-node.sinks.tgck-recorder/v1").requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.text-sample/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn tgck_recorder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.sinks.tgck-recorder/v1",
        );
    }
}
