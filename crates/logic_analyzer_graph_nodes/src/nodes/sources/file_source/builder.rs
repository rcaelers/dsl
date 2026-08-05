//! Runtime builder for `DSL File Source`.

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::RuntimeBuilder;
use logic_analyzer_graph_capabilities::node_support::{
    CaptureCacheIdentity, CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs,
    parse_state,
};
use logic_analyzer_processing::CaptureSourceMetadata;
use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSourceConfig, DslFileSourceFactory, unavailable_source_factory,
};
use node_graph::api::Socket;
use signal_processing::{
    DEFAULT_DERIVED_DATA_MAX_ENTRIES, DerivedDataRetention, Sample, SampleBlock,
};
use signal_runtime::ProcessNode;

pub(crate) struct FileSourceBuilder {
    source_factory: Arc<dyn DslFileSourceFactory>,
}

impl Default for FileSourceBuilder {
    fn default() -> Self {
        Self {
            source_factory: unavailable_source_factory(),
        }
    }
}

impl FileSourceBuilder {
    pub(crate) fn with_source_factory(source_factory: Arc<dyn DslFileSourceFactory>) -> Self {
        Self { source_factory }
    }

    fn config(state: &super::definition::DslFileSourceState) -> DslFileSourceConfig {
        DslFileSourceConfig::new(&state.file.value, state.channel_names.iter().cloned())
    }

    fn metadata(&self, state: &Value) -> Result<Arc<dyn CaptureSourceMetadata>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        Ok(self.source_factory.metadata(Self::config(&state)))
    }
}

pub(crate) fn runtime_builder_override(
    source_factory: Arc<dyn DslFileSourceFactory>,
) -> logic_analyzer_graph_capabilities::node::RuntimeBuilderOverride {
    logic_analyzer_graph_capabilities::node::RuntimeBuilderOverride::new(
        "org.logicconduit.graph-node.sources.dsl-file-source/v1",
        Box::new(FileSourceBuilder::with_source_factory(source_factory)),
    )
}

impl RuntimeBuilder for FileSourceBuilder {
    fn is_source(&self) -> bool {
        true
    }
    fn source_data_lifecycle(
        &self,
    ) -> Option<logic_analyzer_graph_capabilities::node_support::SourceDataLifecycle> {
        Some(super::super::metadata::lifecycle(
            self.source_factory.lifecycle(),
        ))
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
        self.metadata(state)?
            .presentation()
            .map(|presentation| presentation.map(super::super::metadata::presentation))
    }
    fn capture_cache_identity(
        &self,
        state: &Value,
        _resolved: &ResolvedInputs,
    ) -> CaptureCacheIdentity {
        let Ok(metadata) = self.metadata(state) else {
            return CaptureCacheIdentity::Dynamic;
        };
        super::super::metadata::cache_identity(metadata.cache_identity())
    }
    fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
        false
    }
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        self.source_factory
            .create(
                name,
                Self::config(&state),
                ctx.artifact_repository(),
                ctx.work_executor(),
            )
            .map(logic_analyzer_processing::ProcessNodeConstruction::into_process)
            .map_err(|error| format!("cannot open '{}': {error}", state.file.value))
    }
}

#[cfg(test)]
fn platform_parity_builder() -> Box<dyn RuntimeBuilder> {
    Box::new(FileSourceBuilder::with_source_factory(Arc::new(
        crate::nodes::test_support::TestSourceFactory::file(),
    )))
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityBuilderRegistration::new(
        "org.logicconduit.graph-node.sources.dsl-file-source/v1",
        platform_parity_builder,
    )
}

#[cfg(test)]
mod builder_tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use logic_analyzer_processing::{
        CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle,
        CaptureSourcePresentation, ProcessNodeConstruction,
    };
    use node_graph::NodeDef;
    use signal_processing::IndexedCapturePresentation;

    use super::super::definition::DslFileSource;
    use super::*;
    use crate::nodes::test_support::{TestCaptureIndexFactory, TestProcessNode};

    #[derive(Default)]
    struct FakeMetadata {
        path: PathBuf,
        operations: Mutex<Vec<String>>,
        dynamic_identity: bool,
    }

    impl CaptureSourceMetadata for FakeMetadata {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
        }

        fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
            self.operations
                .lock()
                .unwrap()
                .push(format!("presentation:{}", self.path.display()));
            let indexed = IndexedCapturePresentation {
                identity: signal_artifacts::SourceIdentity::from_bytes([0x5A; 32]),
                factory: Box::new(TestCaptureIndexFactory::new(&self.path)),
            };
            Ok(Some(CaptureSourcePresentation::Indexed(indexed)))
        }

        fn cache_identity(&self) -> CaptureSourceCacheIdentity {
            self.operations
                .lock()
                .unwrap()
                .push(format!("identity:{}", self.path.display()));
            if self.dynamic_identity {
                CaptureSourceCacheIdentity::Dynamic
            } else {
                CaptureSourceCacheIdentity::Stable([0x5A; 32])
            }
        }

        fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
            Ok(Some(vec!["Clock".into()]))
        }
    }

    struct FakeSourceFactory {
        opened: Mutex<Vec<String>>,
        error: Option<String>,
        metadata: Arc<FakeMetadata>,
    }

    impl DslFileSourceFactory for FakeSourceFactory {
        fn lifecycle(&self) -> logic_analyzer_processing::CaptureSourceLifecycle {
            self.metadata.lifecycle()
        }

        fn metadata(&self, _config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
            self.metadata.clone()
        }

        fn create(
            &self,
            name: &str,
            config: DslFileSourceConfig,
            _artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
            _work_executor: Arc<dyn signal_runtime::WorkExecutor>,
        ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
            self.opened
                .lock()
                .unwrap()
                .push(format!("{}:{name}", config.path().display()));
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(ProcessNodeConstruction::new(
                Box::new(TestProcessNode::new(name)),
                self.metadata.clone(),
            ))
        }
    }

    #[test]
    fn processing_metadata_drives_lowering_presentation_and_cache_identity() {
        let metadata = Arc::new(FakeMetadata {
            path: PathBuf::from("fixture.dsl"),
            operations: Mutex::default(),
            dynamic_identity: false,
        });
        let source_factory = Arc::new(FakeSourceFactory {
            opened: Mutex::default(),
            error: None,
            metadata: metadata.clone(),
        });
        let builder = FileSourceBuilder::with_source_factory(source_factory.clone());
        let state = fixture_state("fixture.dsl");
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let presentation = builder.capture_presentation(&state).unwrap().unwrap();
        let CapturePresentation::Indexed { identity, factory } = presentation else {
            panic!("file source must publish an indexed presentation");
        };
        assert_eq!(
            identity,
            signal_artifacts::SourceIdentity::from_bytes([0x5A; 32])
        );
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
            &*metadata.operations.lock().unwrap(),
            &["presentation:fixture.dsl", "identity:fixture.dsl"]
        );
    }

    #[test]
    fn factory_failures_are_reported_without_host_files() {
        let metadata = Arc::new(FakeMetadata {
            path: PathBuf::from("missing.dsl"),
            operations: Mutex::default(),
            dynamic_identity: true,
        });
        let source_factory = Arc::new(FakeSourceFactory {
            error: Some("controlled open failure".into()),
            opened: Mutex::default(),
            metadata,
        });
        let builder = FileSourceBuilder::with_source_factory(source_factory);
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
    fn malformed_state_is_rejected_before_factory_access() {
        let metadata = Arc::new(FakeMetadata {
            path: PathBuf::from("unused.dsl"),
            operations: Mutex::default(),
            dynamic_identity: false,
        });
        let source_factory = Arc::new(FakeSourceFactory {
            opened: Mutex::default(),
            error: None,
            metadata: metadata.clone(),
        });
        let builder = FileSourceBuilder::with_source_factory(source_factory.clone());
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
        assert!(metadata.operations.lock().unwrap().is_empty());
        assert!(source_factory.opened.lock().unwrap().is_empty());
    }

    fn fixture_state(path: &str) -> Value {
        let mut state = serde_json::to_value(DslFileSource::state()).unwrap();
        state["file"]["value"] = path.into();
        state
    }
}
