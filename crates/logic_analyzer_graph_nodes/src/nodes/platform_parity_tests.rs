use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;

use logic_analyzer_graph_api::node::{
    GraphNodeRegistration, RuntimeBuilder, graph_node_registrations,
};
use logic_analyzer_graph_api::node_support::{
    CaptureCacheIdentity, CapturePresentation, ResolvedInput, ResolvedInputs,
    SourceDataLifecycleKind,
};
use node_graph::api::{GraphDocumentBuilder, NodeId, NodeTypeRegistry, Socket};

use super::test_support::{TestNodeBuildContext, platform_parity_builder};

const EXPECTATIONS: &str = r###"
{
  "portable_nodes": [
    {
      "stable_id": "org.logicconduit.graph-node.sources.dsl-file-source/v1",
      "name": "DSL File Source",
      "category": "Sources",
      "default_state": {
        "channel_names": [],
        "file": {
          "dialog_title": "Select DSLogic capture",
          "filters": [
            {
              "extensions": ["dsl"],
              "name": "DSLogic captures"
            }
          ],
          "save": false,
          "value": ""
        },
        "metadata_path": ""
      },
      "schema_state_patch": {
        "channel_names": ["Clock", "Data"]
      },
      "inputs": [
        {
          "schema_id": "File",
          "name": "File",
          "type_name": "Text",
          "accepted": []
        }
      ],
      "outputs": {
        "count": 2,
        "schema_id_prefix": "Ch ",
        "names": ["Clock", "Data"],
        "type_name": "Signal",
        "offered": [
          {"kind": "SampleEdge", "port_prefix": "ch"},
          {"kind": "Block", "port_prefix": "ch"}
        ]
      },
      "lifecycle": {
        "kind": "file",
        "preload": true,
        "cache": true,
        "index": true
      },
      "presentation": {"kind": "none"},
      "cache_identity": "dynamic"
    },
    {
      "stable_id": "org.logicconduit.graph-node.sources.dslogic-u3pro16/v1",
      "name": "DSLogic U3Pro16",
      "category": "Sources",
      "default_state": {
        "channels": {
          "enabled": [
            true, true, true, true, true, true, true, true,
            true, true, true, true, true, true, true, true
          ]
        },
        "clock_edge": {
          "value": "Rising",
          "variants": ["Rising", "Falling"]
        },
        "duration": {"nanoseconds": 1000000000},
        "ext_clock": {"value": false},
        "filter": {"value": false},
        "mode": {
          "value": "Stream",
          "variants": ["Stream", "Buffer"]
        },
        "recording_start": {
          "value": "Immediate",
          "variants": ["Immediate", "Trigger"]
        },
        "retention": {
          "value": "Everything",
          "variants": ["Everything", "Recent duration", "Recent bytes"]
        },
        "retention_duration_ms": {
          "max": 2147483647,
          "min": 1,
          "value": 10000
        },
        "retention_megabytes": {
          "max": 2147483647,
          "min": 1,
          "value": 1024
        },
        "rle": {"value": false},
        "sample_rate": {
          "value": "125 MHz",
          "variants": [
            "1 MHz", "2 MHz", "5 MHz", "10 MHz", "20 MHz", "25 MHz",
            "50 MHz", "100 MHz", "125 MHz", "250 MHz", "500 MHz", "1 GHz"
          ]
        },
        "schema_version": 5,
        "summary": {"text": "16 ch @ 125 MHz · 1.0 V"},
        "threshold": {
          "max": 5.0,
          "min": 0.0,
          "speed": 0.05000000074505806,
          "value": 1.0
        },
        "trigger_position_percent": {
          "max": 100,
          "min": 0,
          "value": 50
        },
        "trigger_program": null,
        "trigger_timeout_action": {
          "value": "Disabled",
          "variants": ["Disabled", "Continue waiting", "Stop"]
        },
        "trigger_timeout_ms": {
          "max": 2147483647,
          "min": 1,
          "value": 10000
        }
      },
      "schema_state_patch": {},
      "inputs": [],
      "outputs": {
        "count": 16,
        "schema_id_prefix": "Ch ",
        "name_prefix": "Ch ",
        "type_name": "Signal",
        "offered": [
          {"kind": "Block", "port_prefix": "ch"},
          {"kind": "SampleEdge", "port_prefix": "ch"}
        ]
      },
      "lifecycle": {
        "kind": "live",
        "preload": false,
        "cache": true,
        "index": true
      },
      "presentation": {"kind": "channels", "count": 16},
      "cache_identity": "not_capture"
    },
    {
      "stable_id": "org.logicconduit.graph-node.sources.sigrok-file-source/v1",
      "name": "Sigrok File Source",
      "category": "Sources",
      "default_state": {
        "channel_names": [],
        "demo_data": false,
        "file": {
          "dialog_title": "Select sigrok capture",
          "filters": [
            {
              "extensions": ["sr"],
              "name": "Sigrok captures"
            }
          ],
          "save": false,
          "value": ""
        },
        "metadata_path": ""
      },
      "schema_state_patch": {
        "channel_names": [
          "Ch 0", "Ch 1", "Ch 2", "Ch 3", "Ch 4", "Ch 5", "Ch 6", "Ch 7",
          "Ch 8", "Ch 9", "Ch 10", "Ch 11", "Ch 12", "Ch 13", "Ch 14", "Ch 15"
        ],
        "demo_data": true
      },
      "inputs": [
        {
          "schema_id": "File",
          "name": "File",
          "type_name": "Text",
          "accepted": []
        }
      ],
      "outputs": {
        "count": 16,
        "schema_id_prefix": "Ch ",
        "name_prefix": "Ch ",
        "type_name": "Signal",
        "offered": [
          {"kind": "Block", "port_prefix": "ch"},
          {"kind": "SampleEdge", "port_prefix": "ch"}
        ]
      },
      "lifecycle": {
        "kind": "file",
        "preload": true,
        "cache": true,
        "index": true
      },
      "presentation": {
        "kind": "in_memory",
        "count": 15,
        "excluded": [9]
      },
      "cache_identity": "not_capture"
    },
    {
      "stable_id": "org.logicconduit.graph-node.sinks.file-writer/v1",
      "name": "File Writer",
      "category": "Output",
      "default_state": {
        "filename": {
          "dialog_title": "Save capture as",
          "filters": [],
          "save": true,
          "value": ""
        },
        "index_csv": {"value": true},
        "write_width": {
          "value": "U8 (low byte)",
          "variants": ["U8 (low byte)", "U16 LE", "U32 LE"]
        }
      },
      "schema_state_patch": {},
      "inputs": [
        {
          "schema_id": "Data",
          "name": "Data",
          "type_name": "Words",
          "accepted": [{"kind": "Word", "port": "data"}]
        },
        {
          "schema_id": "Filename",
          "name": "Filename",
          "type_name": "Text",
          "accepted": [{"kind": "Text", "port": "filename"}]
        }
      ],
      "outputs": {"count": 0},
      "lifecycle": null,
      "presentation": {"kind": "none"},
      "cache_identity": "not_capture"
    },
    {
      "stable_id": "org.logicconduit.graph-node.sinks.text-file-writer/v1",
      "name": "Text File Writer",
      "category": "Output",
      "default_state": null,
      "schema_state_patch": {},
      "inputs": [
        {
          "schema_id": "Lines",
          "name": "Lines",
          "type_name": "Text",
          "accepted": [{"kind": "Text", "port": "lines"}]
        },
        {
          "schema_id": "Filename",
          "name": "Filename",
          "type_name": "Text",
          "accepted": [{"kind": "Text", "port": "filename"}]
        }
      ],
      "outputs": {"count": 0},
      "lifecycle": null,
      "presentation": {"kind": "none"},
      "cache_identity": "not_capture"
    },
    {
      "stable_id": "org.logicconduit.graph-node.sinks.csv-writer/v1",
      "name": "CSV Writer",
      "category": "Output",
      "default_state": {
        "filename": {
          "dialog_title": "Save CSV as",
          "filters": [],
          "save": true,
          "value": ""
        },
        "header": {"value": "id,time_ns,value"},
        "hex_digits": {
          "max": 16,
          "min": 1,
          "value": 6
        },
        "value_format": {
          "value": "Decimal",
          "variants": ["Decimal", "Hex"]
        }
      },
      "schema_state_patch": {},
      "inputs": [
        {
          "schema_id": "Data",
          "name": "Data",
          "type_name": "Words",
          "accepted": [{"kind": "Word", "port": "data"}]
        },
        {
          "schema_id": "Filename",
          "name": "Filename",
          "type_name": "Text",
          "accepted": [{"kind": "Text", "port": "filename"}]
        }
      ],
      "outputs": {"count": 0},
      "lifecycle": null,
      "presentation": {"kind": "none"},
      "cache_identity": "not_capture"
    }
  ],
  "registered_nodes": [
    {
      "stable_id": "org.logicconduit.graph-node.decoders.sigrok-decoder/v1",
      "name": "Sigrok Decoder"
    }
  ]
}
"###;

#[derive(Deserialize)]
struct Expectations {
    portable_nodes: Vec<NodeExpectation>,
    registered_nodes: Vec<RegisteredNodeExpectation>,
}

#[derive(Deserialize)]
struct NodeExpectation {
    stable_id: String,
    name: String,
    category: String,
    default_state: Value,
    schema_state_patch: Value,
    inputs: Vec<InputExpectation>,
    outputs: OutputExpectation,
    lifecycle: Option<LifecycleExpectation>,
    presentation: PresentationExpectation,
    cache_identity: String,
}

#[derive(Deserialize)]
struct RegisteredNodeExpectation {
    stable_id: String,
    name: String,
}

#[derive(Deserialize)]
struct InputExpectation {
    schema_id: String,
    name: String,
    type_name: String,
    accepted: Vec<PortExpectation>,
}

#[derive(Deserialize)]
struct OutputExpectation {
    count: usize,
    #[serde(default)]
    schema_id_prefix: String,
    #[serde(default)]
    name_prefix: String,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    type_name: String,
    #[serde(default)]
    offered: Vec<OutputPortExpectation>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct PortExpectation {
    kind: String,
    port: String,
}

#[derive(Deserialize)]
struct OutputPortExpectation {
    kind: String,
    port_prefix: String,
}

#[derive(Deserialize)]
struct LifecycleExpectation {
    kind: String,
    preload: bool,
    cache: bool,
    index: bool,
}

#[derive(Deserialize)]
struct PresentationExpectation {
    kind: String,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    excluded: Vec<usize>,
}

#[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
fn portable_capture_catalog_matches_shared_availability_contract() {
    crate::link();
    let expected = expectations();
    let registrations = graph_node_registrations();

    for node in &expected.portable_nodes {
        let registration = registration(&registrations, &node.stable_id)
            .unwrap_or_else(|| panic!("portable node '{}' is unavailable", node.stable_id));
        assert_eq!(registration.name(), node.name);
    }

    for node in &expected.registered_nodes {
        let actual = registration(&registrations, &node.stable_id)
            .unwrap_or_else(|| panic!("portable node '{}' is unavailable", node.stable_id));
        assert_eq!(actual.name(), node.name);
    }
}

#[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
fn portable_capture_nodes_match_shared_graph_contracts() {
    crate::link();
    let expected = expectations();
    let registrations = graph_node_registrations();

    for expectation in &expected.portable_nodes {
        let registration = registration(&registrations, &expectation.stable_id).unwrap();
        let (mut document, node_id, category) = instantiate(registration);
        let default_state = document.graph().nodes[&node_id].state.clone();
        assert_eq!(
            default_state, expectation.default_state,
            "{} changed its serialized default state",
            expectation.stable_id
        );
        assert_enum_options_are_valid(&default_state, &expectation.stable_id);
        assert_eq!(category, expectation.category);

        let mut schema_state = default_state;
        merge(&mut schema_state, &expectation.schema_state_patch);
        assert!(document.set_node_state(node_id, schema_state));
        let node = &document.graph().nodes[&node_id];
        let builder = registration.builder().expect("portable node is runnable");

        assert_input_contracts(&*builder, &node.inputs, &node.state, expectation);
        assert_output_contracts(&*builder, &node.outputs, &node.state, expectation);
        assert_lifecycle(&*builder, expectation);
        assert_presentation(&*builder, &node.state, expectation);
        assert_cache_identity(&*builder, &node.state, expectation);
    }
}

#[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
fn portable_capture_nodes_lower_through_the_same_neutral_factory_contracts() {
    crate::link();
    let expected = expectations();
    let registrations = graph_node_registrations();

    for expectation in &expected.portable_nodes {
        let registration = registration(&registrations, &expectation.stable_id).unwrap();
        let (mut document, node_id, _) = instantiate(registration);
        let mut state = document.graph().nodes[&node_id].state.clone();
        merge(&mut state, &expectation.schema_state_patch);
        assert!(document.set_node_state(node_id, state));
        let node = &document.graph().nodes[&node_id];
        let builder = builder_with_neutral_factory(&expectation.stable_id);
        let resolved = resolved_inputs(&*builder, &node.inputs, &node.state);
        let mut context = TestNodeBuildContext::default();

        let runtime = builder
            .build(&expectation.name, &node.state, &resolved, &mut context)
            .unwrap_or_else(|error| {
                panic!(
                    "{} failed platform-neutral lowering: {error}",
                    expectation.stable_id
                )
            });
        assert_eq!(runtime.name(), expectation.name);
    }
}

fn expectations() -> Expectations {
    serde_json::from_str(EXPECTATIONS).expect("platform parity expectations are valid JSON")
}

fn registration<'a>(
    registrations: &'a [&GraphNodeRegistration],
    stable_id: &str,
) -> Option<&'a GraphNodeRegistration> {
    registrations
        .iter()
        .copied()
        .find(|registration| registration.stable_id() == stable_id)
}

fn instantiate(registration: &GraphNodeRegistration) -> (GraphDocumentBuilder, NodeId, String) {
    let mut registry = NodeTypeRegistry::new();
    registration.apply_node(&mut registry);
    let category = registry
        .category_of(registration.name())
        .expect("registered node has a category")
        .to_owned();
    let mut document = GraphDocumentBuilder::new(registry);
    let node = document
        .add_node(registration.name())
        .expect("registered node can be instantiated");
    (document, node, category)
}

fn assert_enum_options_are_valid(value: &Value, stable_id: &str) {
    match value {
        Value::Array(values) => {
            for value in values {
                assert_enum_options_are_valid(value, stable_id);
            }
        }
        Value::Object(object) => {
            if let Some(variants) = object.get("variants").and_then(Value::as_array) {
                let selected = object
                    .get("value")
                    .expect("an option list has a selected value");
                assert!(
                    variants.contains(selected),
                    "{stable_id} selects {selected} outside its serialized options {variants:?}"
                );
            }
            for value in object.values() {
                assert_enum_options_are_valid(value, stable_id);
            }
        }
        _ => {}
    }
}

fn assert_input_contracts(
    builder: &dyn RuntimeBuilder,
    sockets: &[Socket],
    state: &Value,
    expectation: &NodeExpectation,
) {
    assert_eq!(
        sockets.len(),
        expectation.inputs.len(),
        "{} input count",
        expectation.stable_id
    );
    for (socket, expected) in sockets.iter().zip(&expectation.inputs) {
        assert_eq!(socket.schema_id, expected.schema_id);
        assert_eq!(socket.name, expected.name);
        assert_eq!(socket.type_name, expected.type_name);
        let actual = builder
            .accepted_kinds(socket, state)
            .into_iter()
            .map(|kind| PortExpectation {
                kind: kind.name().to_owned(),
                port: builder
                    .input_port(socket, 0, state, kind)
                    .expect("accepted input kind resolves to a runtime port"),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected.accepted);
    }
}

fn assert_output_contracts(
    builder: &dyn RuntimeBuilder,
    sockets: &[Socket],
    state: &Value,
    expectation: &NodeExpectation,
) {
    let expected = &expectation.outputs;
    assert_eq!(
        sockets.len(),
        expected.count,
        "{} output count",
        expectation.stable_id
    );
    for (index, socket) in sockets.iter().enumerate() {
        assert_eq!(
            socket.schema_id,
            format!("{}{}", expected.schema_id_prefix, index)
        );
        let expected_name = expected
            .names
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("{}{}", expected.name_prefix, index));
        assert_eq!(socket.name, expected_name);
        assert_eq!(socket.type_name, expected.type_name);
        let actual = builder
            .offered_kinds(socket, state)
            .into_iter()
            .map(|kind| {
                (
                    kind.name().to_owned(),
                    builder
                        .output_port(socket, state, kind)
                        .expect("offered output kind resolves to a runtime port"),
                )
            })
            .collect::<Vec<_>>();
        let expected = expected
            .offered
            .iter()
            .map(|port| (port.kind.clone(), format!("{}{}", port.port_prefix, index)))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

fn assert_lifecycle(builder: &dyn RuntimeBuilder, expectation: &NodeExpectation) {
    let actual = builder.source_data_lifecycle();
    let Some(expected) = &expectation.lifecycle else {
        assert!(actual.is_none());
        return;
    };
    let actual = actual.expect("capture source declares lifecycle metadata");
    let kind = match expected.kind.as_str() {
        "file" => SourceDataLifecycleKind::File,
        "live" => SourceDataLifecycleKind::Live,
        kind => panic!("unsupported lifecycle kind '{kind}'"),
    };
    assert_eq!(actual.kind, kind);
    assert_eq!(actual.preload, expected.preload);
    assert_eq!(actual.cache, expected.cache);
    assert_eq!(actual.index, expected.index);
}

fn assert_presentation(builder: &dyn RuntimeBuilder, state: &Value, node: &NodeExpectation) {
    let expected = &node.presentation;
    let presentation = builder
        .capture_presentation(state)
        .unwrap_or_else(|error| panic!("{} presentation failed: {error}", node.stable_id));
    match (expected.kind.as_str(), presentation) {
        ("none", None) => {}
        ("channels", Some(CapturePresentation::Channels(channels))) => {
            assert_eq!(channels.len(), expected.count);
            for (index, (channel, name)) in channels.iter().enumerate() {
                assert_eq!(*channel, index);
                assert_eq!(name, &format!("Ch {index}"));
            }
        }
        (
            "in_memory",
            Some(CapturePresentation::InMemory {
                signals,
                duration_us,
            }),
        ) => {
            assert_eq!(signals.len(), expected.count);
            assert!(duration_us > 0.0);
            let excluded = expected.excluded.iter().copied().collect::<BTreeSet<_>>();
            let expected_indices = (0..expected.count + excluded.len())
                .filter(|index| !excluded.contains(index))
                .collect::<Vec<_>>();
            assert_eq!(
                signals
                    .iter()
                    .map(|signal| signal.index)
                    .collect::<Vec<_>>(),
                expected_indices
            );
        }
        (kind, _) => panic!(
            "{} did not produce expected {kind} presentation",
            node.stable_id
        ),
    }
}

fn assert_cache_identity(builder: &dyn RuntimeBuilder, state: &Value, node: &NodeExpectation) {
    let actual = builder.capture_cache_identity(state, &ResolvedInputs::default());
    let expected = match node.cache_identity.as_str() {
        "not_capture" => CaptureCacheIdentity::NotCapture,
        "dynamic" => CaptureCacheIdentity::Dynamic,
        identity => panic!("unsupported cache identity '{identity}'"),
    };
    assert_eq!(actual, expected);
}

fn merge(value: &mut Value, patch: &Value) {
    if let (Some(value), Some(patch)) = (value.as_object_mut(), patch.as_object()) {
        for (key, patch) in patch {
            match value.get_mut(key) {
                Some(value) => merge(value, patch),
                None => {
                    value.insert(key.clone(), patch.clone());
                }
            }
        }
    } else {
        *value = patch.clone();
    }
}

fn resolved_inputs(
    builder: &dyn RuntimeBuilder,
    sockets: &[Socket],
    state: &Value,
) -> ResolvedInputs {
    let mut resolved = ResolvedInputs::default();
    for socket in sockets {
        let Some(kind) = builder.accepted_kinds(socket, state).into_iter().next() else {
            continue;
        };
        resolved.insert(
            socket.def_index,
            0,
            ResolvedInput {
                kind,
                source: format!("fixture_{}", socket.def_index),
                source_node: NodeId(10_000 + socket.def_index as u32),
                source_output: socket.def_index,
                source_node_title: "Fixture source".to_owned(),
                word_display_format: None,
                lane_presentation: None,
                default_lane_presentation: None,
                decoder_table_column: None,
                capture_channel: None,
            },
        );
    }
    resolved
}

fn builder_with_neutral_factory(stable_id: &str) -> Box<dyn RuntimeBuilder> {
    platform_parity_builder(stable_id)
}
