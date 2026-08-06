inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::TestCaptureSource>("org.logicconduit.graph-node.sources.test-capture-source/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::TestCaptureSourceBuilder, super::builder::TestCaptureSourceBuilder>(
        "org.logicconduit.graph-node.sources.test-capture-source/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::TestCaptureSource>,
    )
    .with_capture_source::<super::builder::TestCaptureSourceBuilder>()
    .with_presentation::<super::builder::TestCaptureSourceBuilder>()
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    use logic_analyzer_graph_registry::graph_node_registrations;

    #[test]
    fn capture_source_registration_contracts_are_self_consistent() {
        for stable_id in [
            "org.logicconduit.graph-node.sources.test-capture-source/v1",
            "org.logicconduit.graph-node.sources.test-live-capture-source/v1",
        ] {
            crate::nodes::test_support::assert_node_registration_contract(stable_id);
        }
    }

    #[test]
    fn capture_source_registrations_expose_narrow_capability_bundles() {
        for (stable_id, live) in [
            (
                "org.logicconduit.graph-node.sources.test-capture-source/v1",
                false,
            ),
            (
                "org.logicconduit.graph-node.sources.test-live-capture-source/v1",
                true,
            ),
        ] {
            let registration = graph_node_registrations()
                .into_iter()
                .find(|registration| registration.stable_id() == stable_id)
                .unwrap_or_else(|| panic!("missing registration '{stable_id}'"));

            assert!(registration.semantics().is_some());
            assert!(registration.materializer().is_some());
            assert!(registration.capture_source().is_some());
            assert!(registration.presentation().is_some());
            assert_eq!(registration.live_capture().is_some(), live);
        }
    }
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::TestLiveCaptureSource>("org.logicconduit.graph-node.sources.test-live-capture-source/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::live_builder::TestLiveCaptureSourceBuilder, super::live_builder::TestLiveCaptureSourceBuilder>(
        "org.logicconduit.graph-node.sources.test-live-capture-source/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::TestLiveCaptureSource>,
    )
    .with_capture_source::<super::live_builder::TestLiveCaptureSourceBuilder>()
    .with_live_capture::<super::live_builder::TestLiveCaptureSourceBuilder>()
    .with_presentation::<super::live_builder::TestLiveCaptureSourceBuilder>()
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}
