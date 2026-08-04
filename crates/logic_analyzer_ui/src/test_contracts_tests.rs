use logic_analyzer_graph_capabilities::node::RuntimeBuilder;
use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, PortKind,
};
use logic_analyzer_graph_registry::{
    GraphNodeRegistration, PayloadRegistration, graph_node_registrations,
};
use node_graph::{AnySocket, InputDef, NodeDef, NodeTypeRegistry, OutputDef, Socket};
use signal_processing::Word;

pub(crate) const WORD_PRODUCER_NAME: &str = "UI Test Word Producer";

struct WordProducerDefinition;

impl NodeDef for WordProducerDefinition {
    type State = serde_json::Value;

    fn name() -> &'static str {
        WORD_PRODUCER_NAME
    }

    fn category() -> &'static str {
        "UI Tests"
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        Vec::new()
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<AnySocket>("Words")]
    }

    fn state() -> Self::State {
        serde_json::Value::Null
    }
}

#[derive(Default)]
struct WordProducerBuilder;

impl RuntimeBuilder for WordProducerBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &serde_json::Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &serde_json::Value) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn input_port(
        &self,
        _socket: &Socket,
        _member_index: usize,
        _state: &serde_json::Value,
        _kind: PortKind,
    ) -> Option<String> {
        None
    }

    fn output_port(
        &self,
        _socket: &Socket,
        _state: &serde_json::Value,
        kind: PortKind,
    ) -> Option<String> {
        (kind == PortKind::of::<Word>()).then(|| "words".to_owned())
    }
}

struct LegacyViewerDefinition;

impl NodeDef for LegacyViewerDefinition {
    type State = serde_json::Value;

    fn name() -> &'static str {
        "Viewer"
    }

    fn category() -> &'static str {
        "UI Tests"
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<AnySocket>("In").variadic(16)]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        Vec::new()
    }

    fn state() -> Self::State {
        serde_json::Value::Null
    }
}

fn word_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("W", [215, 140, 60]),
        "org.logicconduit.ui-test.word-renderer/v1",
    )
}

inventory::submit! {
    GraphNodeRegistration::runnable::<WordProducerDefinition, WordProducerBuilder>(
        "org.logicconduit.ui-test.word-producer/v1",
    )
}

inventory::submit! {
    GraphNodeRegistration::definition::<LegacyViewerDefinition>(
        crate::viewer_selection::LEGACY_VIEWER_NODE_ID,
    )
}

inventory::submit! {
    PayloadRegistration::subscribable::<Word>(
        "org.logicconduit.word/v1",
        signal_processing::word_payload_adapter,
        word_presentation,
    )
}

pub(crate) fn build_test_node_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in graph_node_registrations() {
        registration.apply_node(&mut registry);
    }
    registry
}
