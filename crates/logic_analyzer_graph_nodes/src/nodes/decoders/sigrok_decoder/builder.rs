use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokChannel, SigrokDecoder, SigrokDecoderConfig, SigrokDecoderDescriptor, SigrokInitialPin,
    SigrokOptionValue, discover_sigrok_decoder,
};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, ProtocolPacket, SampleBlock, Word};

use super::definition::{SavedOptionControl, SavedOutputKind, SavedScalar, SigrokDecoderState};

trait SigrokDecoderBackend: Send + Sync {
    fn discover(
        &self,
        decoder_root: &std::path::Path,
        decoder_id: &str,
    ) -> Result<SigrokDecoderDescriptor, String>;

    fn create(
        &self,
        name: &str,
        config: SigrokDecoderConfig,
    ) -> Result<Box<dyn ProcessNode>, String>;
}

struct PythonSigrokDecoderBackend;

impl SigrokDecoderBackend for PythonSigrokDecoderBackend {
    fn discover(
        &self,
        decoder_root: &std::path::Path,
        decoder_id: &str,
    ) -> Result<SigrokDecoderDescriptor, String> {
        discover_sigrok_decoder(decoder_root, decoder_id)
    }

    fn create(
        &self,
        name: &str,
        config: SigrokDecoderConfig,
    ) -> Result<Box<dyn ProcessNode>, String> {
        SigrokDecoder::new(config)
            .map(|decoder| Box::new(decoder.with_name(name)) as Box<dyn ProcessNode>)
    }
}

pub(crate) struct SigrokDecoderBuilder {
    backend: Arc<dyn SigrokDecoderBackend>,
}

impl Default for SigrokDecoderBuilder {
    fn default() -> Self {
        Self {
            backend: Arc::new(PythonSigrokDecoderBackend),
        }
    }
}

impl SigrokDecoderBuilder {
    fn parsed(state: &Value) -> Result<SigrokDecoderState, String> {
        parse_state(state)
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn SigrokDecoderBackend>) -> Self {
        Self { backend }
    }
}

impl RuntimeBuilder for SigrokDecoderBuilder {
    fn accepted_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind> {
        let Ok(state) = Self::parsed(state) else {
            return Vec::new();
        };
        if socket.def_index == state.channels.len() && !state.protocol_inputs.is_empty() {
            vec![PortKind::of_named::<ProtocolPacket>("Protocol Packet")]
        } else {
            vec![PortKind::of::<SampleBlock>()]
        }
    }

    fn offered_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind> {
        let Ok(state) = Self::parsed(state) else {
            return Vec::new();
        };
        state
            .outputs
            .get(socket.def_index)
            .copied()
            .map(output_kind)
            .into_iter()
            .collect()
    }

    fn offered_connection_contracts(&self, socket: &Socket, state: &Value) -> Vec<String> {
        let Ok(state) = Self::parsed(state) else {
            return Vec::new();
        };
        if state
            .outputs
            .get(socket.def_index)
            .is_some_and(|output| *output == SavedOutputKind::ProtocolPacket)
        {
            state.protocol_outputs
        } else {
            Vec::new()
        }
    }

    fn accepted_connection_contracts(&self, socket: &Socket, state: &Value) -> Vec<String> {
        let Ok(state) = Self::parsed(state) else {
            return Vec::new();
        };
        if socket.def_index == state.channels.len() && !state.protocol_inputs.is_empty() {
            state.protocol_inputs
        } else {
            Vec::new()
        }
    }

    fn input_port(
        &self,
        socket: &Socket,
        _member_index: usize,
        state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        let state = Self::parsed(state).ok()?;
        if socket.def_index == state.channels.len() && !state.protocol_inputs.is_empty() {
            return (kind == PortKind::of_named::<ProtocolPacket>("Protocol Packet"))
                .then(|| "packets".to_owned());
        }
        if kind != PortKind::of::<SampleBlock>() {
            return None;
        }
        state
            .channels
            .get(socket.def_index)
            .map(|channel| channel.id.clone())
    }

    fn output_port(&self, socket: &Socket, state: &Value, kind: PortKind) -> Option<String> {
        let state = Self::parsed(state).ok()?;
        let output = *state.outputs.get(socket.def_index)?;
        (kind == output_kind(output)).then(|| output.port_name().to_owned())
    }

    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        let Ok(state) = Self::parsed(state) else {
            return true;
        };
        if socket.def_index == state.channels.len() && !state.protocol_inputs.is_empty() {
            return true;
        }
        state
            .channels
            .get(socket.def_index)
            .is_none_or(|channel| channel.required)
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state = Self::parsed(state)?;
        if state.decoder_id.is_empty() {
            return Err("No Sigrok decoder is selected".to_owned());
        }
        let current = self
            .backend
            .discover(&state.decoder_root, &state.decoder_id)?;
        if current.package_fingerprint != state.package_fingerprint {
            return Err(format!(
                "Sigrok decoder '{}' changed since this graph was saved; reselect it to migrate its channels and options",
                state.decoder_id
            ));
        }
        validate_descriptor_schema(&state, &current)?;
        let channels = state
            .channels
            .iter()
            .enumerate()
            .map(|(index, channel)| SigrokChannel {
                name: channel.id.clone(),
                connected: resolved.kind(index).is_some(),
                initial_pin: match channel.initial_pin.selected() {
                    "Low" => SigrokInitialPin::Low,
                    "High" => SigrokInitialPin::High,
                    _ => SigrokInitialPin::SameAsFirstSample,
                },
            })
            .collect();
        let options = state
            .options
            .iter()
            .map(|option| Ok((option.id.clone(), option_value(&option.control)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut annotation_rows_by_class = vec![Vec::new(); state.annotation_class_count];
        for (row, descriptor) in state.annotation_rows.iter().enumerate() {
            for &class in &descriptor.classes {
                let Some(rows) = annotation_rows_by_class.get_mut(class) else {
                    return Err(format!(
                        "Sigrok decoder '{}' has an invalid saved annotation class {class}",
                        state.decoder_id
                    ));
                };
                rows.push(row);
            }
        }
        let sample_rate = state.sample_rate()?;
        self.backend.create(
            name,
            SigrokDecoderConfig {
                decoder_root: state.decoder_root,
                decoder_id: state.decoder_id,
                sample_rate,
                channels,
                protocol_inputs: state.protocol_inputs,
                options,
                annotation_rows_by_class: annotation_rows_by_class
                    .into_iter()
                    .map(Arc::from)
                    .collect(),
                binary_class_count: state.binary_class_count,
                logic_groups: state.logic_groups,
            },
        )
    }
}

fn output_kind(output: SavedOutputKind) -> PortKind {
    match output {
        SavedOutputKind::Annotation | SavedOutputKind::Binary | SavedOutputKind::Metadata => {
            PortKind::of::<Word>()
        }
        SavedOutputKind::GeneratedLogic => PortKind::of::<SampleBlock>(),
        SavedOutputKind::ProtocolPacket => PortKind::of_named::<ProtocolPacket>("Protocol Packet"),
    }
}

fn option_value(control: &SavedOptionControl) -> Result<SigrokOptionValue, String> {
    match control {
        SavedOptionControl::Bool(value) => Ok(SigrokOptionValue::Bool(value.value)),
        SavedOptionControl::Integer(value) => {
            Ok(SigrokOptionValue::Integer(i64::from(value.value)))
        }
        SavedOptionControl::Float(value) => Ok(SigrokOptionValue::Float(f64::from(value.value))),
        SavedOptionControl::String(value) => Ok(SigrokOptionValue::String(value.value.clone())),
        SavedOptionControl::Choice { selected, values } => values
            .get(selected.index)
            .ok_or_else(|| "Sigrok decoder option selection is invalid".to_owned())
            .map(scalar_value),
    }
}

fn scalar_value(value: &SavedScalar) -> SigrokOptionValue {
    match value {
        SavedScalar::Bool(value) => SigrokOptionValue::Bool(*value),
        SavedScalar::Integer(value) => SigrokOptionValue::Integer(*value),
        SavedScalar::Float(value) => SigrokOptionValue::Float(*value),
        SavedScalar::String(value) => SigrokOptionValue::String(value.clone()),
    }
}

fn validate_descriptor_schema(
    state: &SigrokDecoderState,
    descriptor: &SigrokDecoderDescriptor,
) -> Result<(), String> {
    let expected = SigrokDecoderState::from_descriptor(state.decoder_root.clone(), descriptor);
    let current_channels = expected
        .channels
        .iter()
        .map(|channel| (channel.id.as_str(), channel.required))
        .collect::<Vec<_>>();
    let saved_channels = state
        .channels
        .iter()
        .map(|channel| (channel.id.as_str(), channel.required))
        .collect::<Vec<_>>();
    if current_channels != saved_channels {
        return Err(format!(
            "Sigrok decoder '{}' channel schema changed; reselect it to migrate the graph",
            state.decoder_id
        ));
    }
    let current_options = expected
        .options
        .iter()
        .map(|option| option.id.as_str())
        .collect::<Vec<_>>();
    let saved_options = state
        .options
        .iter()
        .map(|option| option.id.as_str())
        .collect::<Vec<_>>();
    if current_options != saved_options {
        return Err(format!(
            "Sigrok decoder '{}' option schema changed; reselect it to migrate the graph",
            state.decoder_id
        ));
    }
    if expected.outputs != state.outputs
        || expected.protocol_inputs != state.protocol_inputs
        || expected.protocol_outputs != state.protocol_outputs
        || expected.annotation_class_count != state.annotation_class_count
        || expected.binary_class_count != state.binary_class_count
        || expected.logic_groups != state.logic_groups
    {
        return Err(format!(
            "Sigrok decoder '{}' output schema changed; reselect it to migrate the graph",
            state.decoder_id
        ));
    }
    Ok(())
}

#[cfg(test)]
mod builder_tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use logic_analyzer_graph_api::node_support::ResolvedInput;
    use node_graph::NodeId;

    use super::*;
    use crate::nodes::test_support::{
        TestNodeBuildContext, TestProcessNode, test_sigrok_logic_descriptor,
    };

    struct FakeBackend {
        descriptor: SigrokDecoderDescriptor,
        discovery_error: Option<String>,
        create_error: Option<String>,
        discoveries: Mutex<Vec<(PathBuf, String)>>,
        creation: Mutex<Option<(String, SigrokDecoderConfig)>>,
    }

    impl FakeBackend {
        fn new(descriptor: SigrokDecoderDescriptor) -> Self {
            Self {
                descriptor,
                discovery_error: None,
                create_error: None,
                discoveries: Mutex::new(Vec::new()),
                creation: Mutex::new(None),
            }
        }
    }

    impl SigrokDecoderBackend for FakeBackend {
        fn discover(
            &self,
            decoder_root: &Path,
            decoder_id: &str,
        ) -> Result<SigrokDecoderDescriptor, String> {
            self.discoveries
                .lock()
                .unwrap()
                .push((decoder_root.to_owned(), decoder_id.to_owned()));
            if let Some(error) = &self.discovery_error {
                Err(error.clone())
            } else {
                Ok(self.descriptor.clone())
            }
        }

        fn create(
            &self,
            name: &str,
            config: SigrokDecoderConfig,
        ) -> Result<Box<dyn ProcessNode>, String> {
            *self.creation.lock().unwrap() = Some((name.to_owned(), config));
            if let Some(error) = &self.create_error {
                Err(error.clone())
            } else {
                Ok(Box::new(TestProcessNode::new(name)))
            }
        }
    }

    fn resolved_channels(indices: &[usize]) -> ResolvedInputs {
        let mut resolved = ResolvedInputs::default();
        for &index in indices {
            resolved.insert(
                index,
                0,
                ResolvedInput {
                    kind: PortKind::of::<SampleBlock>(),
                    source: format!("source_{index}"),
                    source_node: NodeId(100 + index as u32),
                    source_output: index,
                    source_node_title: format!("Source {index}"),
                    word_display_format: None,
                    lane_presentation: None,
                    default_lane_presentation: None,
                    decoder_table_column: None,
                    capture_channel: Some(index),
                },
            );
        }
        resolved
    }

    #[test]
    fn saved_descriptor_lowers_through_the_injected_backend() {
        let descriptor = test_sigrok_logic_descriptor();
        let backend = Arc::new(FakeBackend::new(descriptor.clone()));
        let builder = SigrokDecoderBuilder::with_backend(backend.clone());
        let root = PathBuf::from("virtual/sigrok-decoders");
        let state = SigrokDecoderState::from_descriptor(root.clone(), &descriptor);
        let state = serde_json::to_value(state).unwrap();
        let mut context = TestNodeBuildContext::default();
        let resolved = resolved_channels(&[0]);

        let runtime = builder
            .build("Fixture decoder", &state, &resolved, &mut context)
            .unwrap();

        assert_eq!(runtime.name(), "Fixture decoder");
        assert_eq!(
            &*backend.discoveries.lock().unwrap(),
            &[(root.clone(), "test_logic".to_owned())]
        );
        let creation = backend.creation.lock().unwrap();
        let (name, config) = creation.as_ref().expect("runtime creation was requested");
        assert_eq!(name, "Fixture decoder");
        assert_eq!(config.decoder_root, root);
        assert_eq!(config.decoder_id, "test_logic");
        assert_eq!(config.sample_rate, 1_000_000);
        assert_eq!(
            config
                .channels
                .iter()
                .map(|channel| (channel.name.as_str(), channel.connected))
                .collect::<Vec<_>>(),
            [("mosi", true), ("cs", false)]
        );
        assert!(config.protocol_inputs.is_empty());
        assert_eq!(config.annotation_rows_by_class.len(), 1);
        assert_eq!(&*config.annotation_rows_by_class[0], &[0]);
        assert_eq!(config.binary_class_count, 1);
        assert_eq!(config.logic_groups, ["Generated"]);
    }

    #[test]
    fn optional_channel_presence_follows_the_graph_connection() {
        let descriptor = test_sigrok_logic_descriptor();
        let backend = Arc::new(FakeBackend::new(descriptor.clone()));
        let builder = SigrokDecoderBuilder::with_backend(backend.clone());
        let state = serde_json::to_value(SigrokDecoderState::from_descriptor(
            PathBuf::from("virtual/sigrok-decoders"),
            &descriptor,
        ))
        .unwrap();
        let mut context = TestNodeBuildContext::default();
        let resolved = resolved_channels(&[0, 1]);

        builder
            .build("Fixture decoder", &state, &resolved, &mut context)
            .unwrap();

        let creation = backend.creation.lock().unwrap();
        let (_, config) = creation.as_ref().expect("runtime creation was requested");
        assert_eq!(
            config
                .channels
                .iter()
                .map(|channel| (channel.name.as_str(), channel.connected))
                .collect::<Vec<_>>(),
            [("mosi", true), ("cs", true)]
        );
    }

    #[test]
    fn discovery_and_runtime_failures_remain_separate() {
        let descriptor = test_sigrok_logic_descriptor();
        let root = PathBuf::from("virtual/sigrok-decoders");
        let state =
            serde_json::to_value(SigrokDecoderState::from_descriptor(root, &descriptor)).unwrap();
        let mut context = TestNodeBuildContext::default();

        let discovery_backend = Arc::new(FakeBackend {
            discovery_error: Some("controlled discovery failure".into()),
            ..FakeBackend::new(descriptor.clone())
        });
        let discovery_error = SigrokDecoderBuilder::with_backend(discovery_backend.clone())
            .build(
                "Fixture decoder",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("discovery failure must be preserved");
        assert_eq!(discovery_error, "controlled discovery failure");
        assert!(discovery_backend.creation.lock().unwrap().is_none());

        let runtime_backend = Arc::new(FakeBackend {
            create_error: Some("controlled runtime failure".into()),
            ..FakeBackend::new(descriptor)
        });
        let runtime_error = SigrokDecoderBuilder::with_backend(runtime_backend.clone())
            .build(
                "Fixture decoder",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("runtime failure must be preserved");
        assert_eq!(runtime_error, "controlled runtime failure");
        assert!(runtime_backend.creation.lock().unwrap().is_some());
    }

    #[test]
    fn changed_package_is_rejected_before_runtime_creation() {
        let saved = test_sigrok_logic_descriptor();
        let mut changed = saved.clone();
        changed.package_fingerprint = "changed-fingerprint".into();
        let backend = Arc::new(FakeBackend::new(changed));
        let builder = SigrokDecoderBuilder::with_backend(backend.clone());
        let state = serde_json::to_value(SigrokDecoderState::from_descriptor(
            PathBuf::from("virtual/sigrok-decoders"),
            &saved,
        ))
        .unwrap();
        let mut context = TestNodeBuildContext::default();

        let error = builder
            .build(
                "Changed decoder",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("changed package must fail");

        assert!(error.contains("changed since this graph was saved"));
        assert!(backend.creation.lock().unwrap().is_none());
    }

    #[test]
    fn malformed_state_is_rejected_before_discovery() {
        let backend = Arc::new(FakeBackend::new(test_sigrok_logic_descriptor()));
        let builder = SigrokDecoderBuilder::with_backend(backend.clone());
        let mut context = TestNodeBuildContext::default();

        let error = builder
            .build(
                "Malformed decoder",
                &Value::Null,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("malformed state must fail");

        assert!(error.starts_with("invalid node state:"));
        assert!(backend.discoveries.lock().unwrap().is_empty());
        assert!(backend.creation.lock().unwrap().is_none());
    }
}
