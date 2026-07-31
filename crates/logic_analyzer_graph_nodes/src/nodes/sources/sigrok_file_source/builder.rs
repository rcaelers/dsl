//! Runtime builder for `Sigrok File Source`.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    CaptureCacheIdentity, CapturePresentation, CapturePresentationSignal, NodeBuildContext,
    PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSourceConfig, SigrokFileSourceFactory, source_factory,
};
use logic_analyzer_processing::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;
use node_graph::api::Socket;
use signal_processing::{ProcessNode, Sample, SampleBlock};

pub(crate) trait SigrokFileArtifacts: Send + Sync {
    fn capture_presentation(
        &self,
        path: &Path,
        channel_names: &[String],
    ) -> Result<Option<CapturePresentation>, String>;
    fn cache_identity(&self, path: &Path) -> Result<[u8; 32], String>;
}

pub(crate) struct SigrokFileSourceBuilder {
    source_factory: Arc<dyn SigrokFileSourceFactory>,
    artifacts: Arc<dyn SigrokFileArtifacts>,
}

impl Default for SigrokFileSourceBuilder {
    fn default() -> Self {
        Self {
            source_factory: source_factory(),
            artifacts: super::metadata_platform::artifacts(),
        }
    }
}

#[cfg(test)]
impl SigrokFileSourceBuilder {
    fn with_dependencies(
        source_factory: Arc<dyn SigrokFileSourceFactory>,
        artifacts: Arc<dyn SigrokFileArtifacts>,
    ) -> Self {
        Self {
            source_factory,
            artifacts,
        }
    }
}

impl RuntimeBuilder for SigrokFileSourceBuilder {
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
    fn viewer_channel_origin(&self, socket: &Socket, _state: &Value) -> Option<usize> {
        Some(socket.def_index)
    }
    fn capture_presentation(&self, state: &Value) -> Result<Option<CapturePresentation>, String> {
        let state: super::definition::SigrokFileSourceState = parse_state(state)?;
        if state.demo_data {
            let channels =
                SyntheticCaptureSource::preview_channels_with_count(state.channel_count());
            let signals = channels
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != 9)
                .map(|(index, samples)| CapturePresentationSignal {
                    index,
                    name: format!("Ch {index}"),
                    initial: samples.first().is_some_and(|sample| sample.value),
                    transitions: samples
                        .iter()
                        .skip(1)
                        .map(|sample| (sample.start_time_ns as f64 / 1_000.0, sample.value))
                        .collect(),
                })
                .collect::<Vec<_>>();
            let duration_us = signals
                .iter()
                .filter_map(|signal| signal.transitions.last().map(|(time, _)| *time))
                .fold(1.0_f64, f64::max);
            return Ok(Some(CapturePresentation::InMemory {
                signals,
                duration_us,
            }));
        }
        let path = std::path::PathBuf::from(state.file.value);
        self.artifacts
            .capture_presentation(&path, &state.channel_names)
    }
    fn capture_cache_identity(
        &self,
        state: &Value,
        _resolved: &ResolvedInputs,
    ) -> CaptureCacheIdentity {
        let Ok(state) = parse_state::<super::definition::SigrokFileSourceState>(state) else {
            return CaptureCacheIdentity::Dynamic;
        };
        if state.demo_data {
            return CaptureCacheIdentity::NotCapture;
        }
        self.artifacts
            .cache_identity(Path::new(&state.file.value))
            .map(CaptureCacheIdentity::Stable)
            .unwrap_or(CaptureCacheIdentity::Dynamic)
    }
    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        socket.def_index == 0
            && parse_state::<super::definition::SigrokFileSourceState>(state)
                .map(|state| !state.demo_data && state.file.value.trim().is_empty())
                .unwrap_or(true)
    }
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::SigrokFileSourceState = parse_state(state)?;
        self.source_factory
            .create(
                name,
                SigrokFileSourceConfig::new(
                    &state.file.value,
                    state.channel_count(),
                    state.demo_data,
                ),
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

    use super::super::definition::SigrokFileSource;
    use super::*;
    use crate::nodes::test_support::{TestCaptureIndexFactory, TestProcessNode};

    #[derive(Default)]
    struct FakeArtifacts {
        operations: Mutex<Vec<String>>,
        identity_error: bool,
    }

    impl SigrokFileArtifacts for FakeArtifacts {
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
                Ok([0xA5; 32])
            }
        }
    }

    #[derive(Default)]
    struct FakeSourceFactory {
        opened: Mutex<Vec<(String, SigrokFileSourceConfig)>>,
        error: Option<String>,
    }

    impl SigrokFileSourceFactory for FakeSourceFactory {
        fn create(
            &self,
            name: &str,
            config: SigrokFileSourceConfig,
        ) -> Result<ProcessNodeConstruction, String> {
            self.opened.lock().unwrap().push((name.to_owned(), config));
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
    fn non_demo_artifacts_drive_lowering_presentation_and_cache_identity() {
        let artifacts = Arc::new(FakeArtifacts::default());
        let source_factory = Arc::new(FakeSourceFactory::default());
        let builder =
            SigrokFileSourceBuilder::with_dependencies(source_factory.clone(), artifacts.clone());
        let state = fixture_state("fixture.sr");
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let presentation = builder.capture_presentation(&state).unwrap().unwrap();
        let CapturePresentation::Indexed { identity, factory } = presentation else {
            panic!("file source must publish an indexed presentation");
        };
        assert_eq!(identity, PathBuf::from("fixture.sr"));
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
            &*artifacts.operations.lock().unwrap(),
            &["presentation:fixture.sr", "identity:fixture.sr"]
        );
    }

    #[test]
    fn non_demo_artifact_failures_are_deterministic() {
        let artifacts = Arc::new(FakeArtifacts {
            identity_error: true,
            ..FakeArtifacts::default()
        });
        let source_factory = Arc::new(FakeSourceFactory {
            error: Some("controlled session failure".into()),
            ..FakeSourceFactory::default()
        });
        let builder = SigrokFileSourceBuilder::with_dependencies(source_factory, artifacts);
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
    fn demo_data_bypasses_file_artifacts() {
        let artifacts = Arc::new(FakeArtifacts::default());
        let source_factory = Arc::new(FakeSourceFactory::default());
        let builder =
            SigrokFileSourceBuilder::with_dependencies(source_factory.clone(), artifacts.clone());
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
        assert!(artifacts.operations.lock().unwrap().is_empty());
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
