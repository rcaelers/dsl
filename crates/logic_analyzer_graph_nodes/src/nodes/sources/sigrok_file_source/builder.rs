//! Runtime builder for `Sigrok File Source`.

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    CaptureSourceFeature, GraphNodeCapabilityOverride, GraphNodePresentation, GraphNodeSemantics,
    RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    CaptureCacheIdentity, CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs,
    parse_state,
};
use logic_analyzer_processing::CaptureSourceMetadata;
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};
use node_graph::api::Socket;
use signal_capture::{Sample, SampleBlock};
use signal_runtime::ProcessNode;

pub(crate) struct SigrokFileSourceBuilder {
    source_factory: Arc<dyn SigrokFileSourceFactory>,
}

impl Default for SigrokFileSourceBuilder {
    fn default() -> Self {
        Self {
            source_factory: portable_source_factory(),
        }
    }
}

impl SigrokFileSourceBuilder {
    pub(crate) fn with_source_factory(source_factory: Arc<dyn SigrokFileSourceFactory>) -> Self {
        Self { source_factory }
    }

    fn config(state: &super::definition::SigrokFileSourceState) -> SigrokFileSourceConfig {
        SigrokFileSourceConfig::new(
            &state.file.value,
            state.channel_names.iter().cloned(),
            state.demo_data,
        )
    }

    fn metadata(&self, state: &Value) -> Result<Arc<dyn CaptureSourceMetadata>, String> {
        let state: super::definition::SigrokFileSourceState = parse_state(state)?;
        Ok(self.source_factory.metadata(Self::config(&state)))
    }
}

pub(crate) fn capability_override(
    source_factory: Arc<dyn SigrokFileSourceFactory>,
) -> GraphNodeCapabilityOverride {
    let stable_id = "org.logicconduit.graph-node.sources.sigrok-file-source/v1";
    GraphNodeCapabilityOverride::capabilities(stable_id)
        .with_semantics(Box::new(SigrokFileSourceBuilder::with_source_factory(
            Arc::clone(&source_factory),
        )))
        .with_materializer(Box::new(SigrokFileSourceBuilder::with_source_factory(
            Arc::clone(&source_factory),
        )))
        .with_capture_source(Box::new(SigrokFileSourceBuilder::with_source_factory(
            Arc::clone(&source_factory),
        )))
        .with_presentation(Box::new(SigrokFileSourceBuilder::with_source_factory(
            source_factory,
        )))
}

impl GraphNodeSemantics for SigrokFileSourceBuilder {
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
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<SampleBlock>(), PortKind::of::<Sample>()]
    }
    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn output_port(&self, socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        (kind == PortKind::of::<Sample>() || kind == PortKind::of::<SampleBlock>())
            .then(|| format!("ch{}", socket.def_index))
    }
    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        socket.def_index == 0
            && parse_state::<super::definition::SigrokFileSourceState>(state)
                .map(|state| !state.demo_data && state.file.value.trim().is_empty())
                .unwrap_or(true)
    }
}

impl CaptureSourceFeature for SigrokFileSourceBuilder {
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
}

impl GraphNodePresentation for SigrokFileSourceBuilder {
    fn viewer_channel_origin(&self, socket: &Socket, _state: &Value) -> Option<usize> {
        Some(socket.def_index)
    }
}

impl RuntimeMaterializer for SigrokFileSourceBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::SigrokFileSourceState = parse_state(state)?;
        self.source_factory
            .create(name, Self::config(&state), ctx.work_executor())
            .map(logic_analyzer_processing::ProcessNodeConstruction::into_process)
            .map_err(|error| format!("cannot open '{}': {error}", state.file.value))
    }
}

#[cfg(test)]
fn platform_parity_capabilities() -> crate::nodes::test_support::PlatformParityCapabilities {
    let factory: Arc<dyn SigrokFileSourceFactory> =
        Arc::new(crate::nodes::test_support::TestSourceFactory::file());
    crate::nodes::test_support::PlatformParityCapabilities::new(
        Box::new(SigrokFileSourceBuilder::with_source_factory(Arc::clone(
            &factory,
        ))),
        Box::new(SigrokFileSourceBuilder::with_source_factory(Arc::clone(
            &factory,
        ))),
    )
    .with_capture_source(Box::new(SigrokFileSourceBuilder::with_source_factory(
        Arc::clone(&factory),
    )))
    .with_presentation(Box::new(SigrokFileSourceBuilder::with_source_factory(
        factory,
    )))
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityCapabilityRegistration::new(
        "org.logicconduit.graph-node.sources.sigrok-file-source/v1",
        platform_parity_capabilities,
    )
}

#[cfg(test)]
mod builder_tests {
    use std::path::Path;
    use std::sync::Mutex;

    use logic_analyzer_processing::{
        CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle,
        CaptureSourcePresentation, ProcessNodeConstruction,
    };
    use node_graph::NodeDef;
    use signal_capture::IndexedCapturePresentation;

    use super::super::definition::SigrokFileSource;
    use super::*;
    use crate::nodes::test_support::{TestCaptureIndexFactory, TestProcessNode};

    struct FakeMetadata {
        config: SigrokFileSourceConfig,
        operations: Arc<Mutex<Vec<String>>>,
        dynamic_identity: bool,
    }

    impl CaptureSourceMetadata for FakeMetadata {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
        }

        fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
            if self.config.demo_data() {
                return Ok(Some(CaptureSourcePresentation::InMemory {
                    signals: Vec::new(),
                    duration_us: 1.0,
                }));
            }
            self.operations
                .lock()
                .unwrap()
                .push(format!("presentation:{}", self.config.path().display()));
            let indexed = IndexedCapturePresentation {
                identity: platform_artifacts::SourceIdentity::from_bytes([0x5B; 32]),
                factory: Box::new(TestCaptureIndexFactory::new(self.config.path())),
            };
            Ok(Some(CaptureSourcePresentation::Indexed(indexed)))
        }

        fn cache_identity(&self) -> CaptureSourceCacheIdentity {
            if self.config.demo_data() {
                return CaptureSourceCacheIdentity::NotCapture;
            }
            self.operations
                .lock()
                .unwrap()
                .push(format!("identity:{}", self.config.path().display()));
            if self.dynamic_identity {
                CaptureSourceCacheIdentity::Dynamic
            } else {
                CaptureSourceCacheIdentity::Stable([0xA5; 32])
            }
        }

        fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
            Ok(Some(self.config.channel_names().to_vec()))
        }
    }

    #[derive(Default)]
    struct FakeSourceFactory {
        opened: Mutex<Vec<(String, SigrokFileSourceConfig)>>,
        error: Option<String>,
        operations: Arc<Mutex<Vec<String>>>,
        dynamic_identity: bool,
    }

    impl SigrokFileSourceFactory for FakeSourceFactory {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
        }

        fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
            Arc::new(FakeMetadata {
                config,
                operations: self.operations.clone(),
                dynamic_identity: self.dynamic_identity,
            })
        }

        fn create(
            &self,
            name: &str,
            config: SigrokFileSourceConfig,
            _work_executor: Arc<dyn platform_runtime::WorkExecutor>,
        ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
            self.opened
                .lock()
                .unwrap()
                .push((name.to_owned(), config.clone()));
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(ProcessNodeConstruction::new(
                Box::new(TestProcessNode::new(name)),
                self.metadata(config),
            ))
        }
    }

    #[test]
    fn non_demo_metadata_drives_lowering_presentation_and_cache_identity() {
        let source_factory = Arc::new(FakeSourceFactory::default());
        let builder = SigrokFileSourceBuilder::with_source_factory(source_factory.clone());
        let state = fixture_state("fixture.sr");
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let presentation = builder.capture_presentation(&state).unwrap().unwrap();
        let CapturePresentation::Indexed { identity, factory } = presentation else {
            panic!("file source must publish an indexed presentation");
        };
        assert_eq!(
            identity,
            platform_artifacts::SourceIdentity::from_bytes([0x5B; 32])
        );
        assert_eq!(factory.display_name(), "fixture.sr");
        assert_eq!(
            builder.capture_cache_identity(&state, &ResolvedInputs::default()),
            CaptureCacheIdentity::Stable([0xA5; 32])
        );
        let runtime = builder
            .build(
                "Fixture session",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .unwrap();
        assert_eq!(runtime.name(), "Fixture session");
        let opened = source_factory.opened.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].0, "Fixture session");
        assert_eq!(opened[0].1.path(), Path::new("fixture.sr"));
        assert!(!opened[0].1.demo_data());
        drop(opened);
        assert_eq!(
            &*source_factory.operations.lock().unwrap(),
            &["presentation:fixture.sr", "identity:fixture.sr"]
        );
    }

    #[test]
    fn non_demo_factory_failures_are_deterministic() {
        let source_factory = Arc::new(FakeSourceFactory {
            error: Some("controlled session failure".into()),
            dynamic_identity: true,
            ..FakeSourceFactory::default()
        });
        let builder = SigrokFileSourceBuilder::with_source_factory(source_factory);
        let state = fixture_state("missing.sr");
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        assert_eq!(
            builder.capture_cache_identity(&state, &ResolvedInputs::default()),
            CaptureCacheIdentity::Dynamic
        );
        let error = builder
            .build(
                "Missing session",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("controlled open failure must be preserved");
        assert_eq!(
            error,
            "cannot open 'missing.sr': controlled session failure"
        );
    }

    #[test]
    fn demo_data_uses_processing_metadata_without_file_access() {
        let source_factory = Arc::new(FakeSourceFactory::default());
        let builder = SigrokFileSourceBuilder::with_source_factory(source_factory.clone());
        let mut state = SigrokFileSource::state();
        state.demo_data = true;
        state.channel_names = vec!["Demo".into()];
        let state = serde_json::to_value(state).unwrap();
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        assert!(matches!(
            builder.capture_presentation(&state).unwrap(),
            Some(CapturePresentation::InMemory { .. })
        ));
        assert_eq!(
            builder.capture_cache_identity(&state, &ResolvedInputs::default()),
            CaptureCacheIdentity::NotCapture
        );
        let runtime = builder
            .build(
                "Demo session",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .unwrap();
        assert_eq!(runtime.name(), "Demo session");
        assert!(source_factory.operations.lock().unwrap().is_empty());
        let opened = source_factory.opened.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert!(opened[0].1.demo_data());
    }

    fn fixture_state(path: &str) -> Value {
        let mut state = SigrokFileSource::state();
        state.file.value = path.into();
        state.channel_names = vec!["Clock".into()];
        serde_json::to_value(state).unwrap()
    }
}
