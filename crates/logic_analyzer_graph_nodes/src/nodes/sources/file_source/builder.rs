//! Runtime builder for `DSL File Source`.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    CaptureCacheIdentity, CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs,
    parse_state,
};
use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSourceConfig, DslFileSourceFactory, source_factory,
};
use node_graph::api::Socket;
use signal_processing::{
    DEFAULT_DERIVED_DATA_MAX_ENTRIES, DerivedDataRetention, ProcessNode, Sample, SampleBlock,
};

pub(crate) trait DslFileArtifacts: Send + Sync {
    fn capture_presentation(
        &self,
        path: &Path,
        channel_names: &[String],
    ) -> Result<Option<CapturePresentation>, String>;
    fn cache_identity(&self, path: &Path) -> Result<[u8; 32], String>;
}

pub(crate) struct FileSourceBuilder {
    source_factory: Arc<dyn DslFileSourceFactory>,
    artifacts: Arc<dyn DslFileArtifacts>,
}

impl Default for FileSourceBuilder {
    fn default() -> Self {
        Self {
            source_factory: source_factory(),
            artifacts: super::metadata_platform::artifacts(),
        }
    }
}

#[cfg(test)]
impl FileSourceBuilder {
    fn with_dependencies(
        source_factory: Arc<dyn DslFileSourceFactory>,
        artifacts: Arc<dyn DslFileArtifacts>,
    ) -> Self {
        Self {
            source_factory,
            artifacts,
        }
    }
}

impl RuntimeBuilder for FileSourceBuilder {
    fn is_source(&self) -> bool {
        true
    }
    fn source_data_lifecycle(
        &self,
    ) -> Option<logic_analyzer_graph_api::node_support::SourceDataLifecycle> {
        Some(
            logic_analyzer_graph_api::node_support::SourceDataLifecycle::new(
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::File,
                true,
                true,
                true,
            ),
        )
    }
    fn derived_data_retention(&self, _state: &Value) -> DerivedDataRetention {
        DerivedDataRetention::MaxEntries(DEFAULT_DERIVED_DATA_MAX_ENTRIES)
    }
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>(), PortKind::of::<SampleBlock>()]
    }
    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn output_port(&self, socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        let channel = socket.def_index;
        // The runtime negotiates Sample vs SampleBlock per connection on a
        // single `ch{channel}` port now — both kinds resolve to the same
        // port name here.
        if kind == PortKind::of::<Sample>() || kind == PortKind::of::<SampleBlock>() {
            Some(format!("ch{channel}"))
        } else {
            None
        }
    }
    fn viewer_channel_origin(&self, socket: &Socket, _state: &Value) -> Option<usize> {
        Some(socket.def_index)
    }
    fn capture_presentation(&self, state: &Value) -> Result<Option<CapturePresentation>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        let path = std::path::PathBuf::from(state.file.value);
        self.artifacts
            .capture_presentation(&path, &state.channel_names)
    }
    fn capture_cache_identity(
        &self,
        state: &Value,
        _resolved: &ResolvedInputs,
    ) -> CaptureCacheIdentity {
        let Ok(state) = parse_state::<super::definition::DslFileSourceState>(state) else {
            return CaptureCacheIdentity::Dynamic;
        };
        if state.file.value.trim().is_empty() {
            return CaptureCacheIdentity::Dynamic;
        }
        self.artifacts
            .cache_identity(Path::new(&state.file.value))
            .map(CaptureCacheIdentity::Stable)
            .unwrap_or(CaptureCacheIdentity::Dynamic)
    }
    fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
        false
    }
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        self.source_factory
            .create(
                name,
                DslFileSourceConfig::new(&state.file.value, state.channel_names.len()),
            )
            .map(logic_analyzer_processing::ProcessNodeConstruction::into_process)
            .map_err(|error| format!("cannot open '{}': {error}", state.file.value))
    }
}

#[cfg(test)]
mod builder_tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use logic_analyzer_processing::ProcessNodeConstruction;
    use node_graph::NodeDef;
    use signal_processing::IndexedCapturePresentation;

    use super::super::definition::DslFileSource;
    use super::*;
    use crate::nodes::test_support::{TestCaptureIndexFactory, TestProcessNode};

    #[derive(Default)]
    struct FakeArtifacts {
        operations: Mutex<Vec<String>>,
        identity_error: bool,
    }

    impl DslFileArtifacts for FakeArtifacts {
        fn capture_presentation(
            &self,
            path: &Path,
            _channel_names: &[String],
        ) -> Result<Option<CapturePresentation>, String> {
            self.operations
                .lock()
                .unwrap()
                .push(format!("presentation:{}", path.display()));
            let indexed = IndexedCapturePresentation {
                identity: path.to_owned(),
                factory: Box::new(TestCaptureIndexFactory::new(path)),
            };
            Ok(Some(CapturePresentation::Indexed {
                identity: indexed.identity,
                factory: indexed.factory,
            }))
        }

        fn cache_identity(&self, path: &Path) -> Result<[u8; 32], String> {
            self.operations
                .lock()
                .unwrap()
                .push(format!("identity:{}", path.display()));
            if self.identity_error {
                Err("controlled identity failure".into())
            } else {
                Ok([0x5A; 32])
            }
        }
    }

    #[derive(Default)]
    struct FakeSourceFactory {
        opened: Mutex<Vec<String>>,
        error: Option<String>,
    }

    impl DslFileSourceFactory for FakeSourceFactory {
        fn create(
            &self,
            name: &str,
            config: DslFileSourceConfig,
        ) -> Result<ProcessNodeConstruction, String> {
            self.opened
                .lock()
                .unwrap()
                .push(format!("{}:{name}", config.path().display()));
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(ProcessNodeConstruction::new(
                Box::new(TestProcessNode::new(name)),
                (),
            ))
        }
    }

    #[test]
    fn artifact_backend_drives_lowering_presentation_and_cache_identity() {
        let artifacts = Arc::new(FakeArtifacts::default());
        let source_factory = Arc::new(FakeSourceFactory::default());
        let builder =
            FileSourceBuilder::with_dependencies(source_factory.clone(), artifacts.clone());
        let state = fixture_state("fixture.dsl");
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let presentation = builder.capture_presentation(&state).unwrap().unwrap();
        let CapturePresentation::Indexed { identity, factory } = presentation else {
            panic!("file source must publish an indexed presentation");
        };
        assert_eq!(identity, PathBuf::from("fixture.dsl"));
        assert_eq!(factory.display_name(), "fixture.dsl");
        assert_eq!(
            builder.capture_cache_identity(&state, &ResolvedInputs::default()),
            CaptureCacheIdentity::Stable([0x5A; 32])
        );
        let runtime = builder
            .build(
                "Fixture file",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .unwrap();
        assert_eq!(runtime.name(), "Fixture file");
        assert_eq!(
            &*source_factory.opened.lock().unwrap(),
            &["fixture.dsl:Fixture file"]
        );
        assert_eq!(
            &*artifacts.operations.lock().unwrap(),
            &["presentation:fixture.dsl", "identity:fixture.dsl"]
        );
    }

    #[test]
    fn artifact_failures_are_reported_without_host_files() {
        let artifacts = Arc::new(FakeArtifacts {
            identity_error: true,
            ..FakeArtifacts::default()
        });
        let source_factory = Arc::new(FakeSourceFactory {
            error: Some("controlled open failure".into()),
            ..FakeSourceFactory::default()
        });
        let builder = FileSourceBuilder::with_dependencies(source_factory, artifacts);
        let state = fixture_state("missing.dsl");
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        assert_eq!(
            builder.capture_cache_identity(&state, &ResolvedInputs::default()),
            CaptureCacheIdentity::Dynamic
        );
        let error = builder
            .build(
                "Missing file",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("controlled open failure must be preserved");
        assert_eq!(error, "cannot open 'missing.dsl': controlled open failure");
    }

    #[test]
    fn malformed_state_is_rejected_before_artifact_access() {
        let artifacts = Arc::new(FakeArtifacts::default());
        let source_factory = Arc::new(FakeSourceFactory::default());
        let builder =
            FileSourceBuilder::with_dependencies(source_factory.clone(), artifacts.clone());
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let error = builder
            .build(
                "Malformed file",
                &Value::Null,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("malformed state must fail");
        assert!(error.starts_with("invalid node state:"));
        assert!(artifacts.operations.lock().unwrap().is_empty());
        assert!(source_factory.opened.lock().unwrap().is_empty());
    }

    fn fixture_state(path: &str) -> Value {
        let mut state = serde_json::to_value(DslFileSource::state()).unwrap();
        state["file"]["value"] = path.into();
        state
    }
}
