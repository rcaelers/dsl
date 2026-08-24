//! Host-facing construction of concrete node-runtime overrides.

use std::sync::Arc;

use logic_analyzer_capture_formats::dsl_file::DslFileSourceFactory;
use logic_analyzer_capture_formats::sigrok_file::SigrokFileSourceFactory;
use logic_analyzer_device_dslogic::DsLogicU3Pro16SourceFactory;
use logic_analyzer_graph_capabilities::node::GraphNodeCapabilityOverride;
use logic_analyzer_graph_editor_registry::GraphNodeEditorOverride;
use logic_analyzer_protocol_decoders::sigrok_decoder::{
    SigrokCatalogScanner, SigrokCatalogSnapshot, SigrokDecoderRuntime,
};
use signal_sinks::binary_file_writer::BinaryFileWriterFactory;
use signal_sinks::csv_word_writer::CsvWordWriterFactory;
use signal_sinks::text_file_writer::TextFileWriterFactory;

const DSL_FILE_SOURCE_ID: &str = "org.logicconduit.graph-node.sources.dsl-file-source/v1";
const SIGROK_FILE_SOURCE_ID: &str = "org.logicconduit.graph-node.sources.sigrok-file-source/v1";
const SIGROK_DECODER_ID: &str = "org.logicconduit.graph-node.decoders.sigrok-decoder/v1";

/// Binds DSL source metadata discovery to one editor registry.
pub fn dsl_file_source_editor_override(
    source_factory: Arc<dyn DslFileSourceFactory>,
) -> GraphNodeEditorOverride {
    GraphNodeEditorOverride::new(DSL_FILE_SOURCE_ID, move |registry| {
        let source_factory = Arc::clone(&source_factory);
        registry
            .register_with_state_update::<
                crate::nodes::sources::file_source::definition::DslFileSource,
                _,
            >(
                move |state, _inputs, _outputs| {
                    crate::nodes::sources::file_source::definition::update_source_metadata(
                        state,
                        source_factory.as_ref(),
                    );
                },
            );
    })
}

/// Binds Sigrok file-source metadata discovery to one editor registry.
pub fn sigrok_file_source_editor_override(
    source_factory: Arc<dyn SigrokFileSourceFactory>,
) -> GraphNodeEditorOverride {
    GraphNodeEditorOverride::new(SIGROK_FILE_SOURCE_ID, move |registry| {
        let source_factory = Arc::clone(&source_factory);
        registry.register_with_state_update::<
            crate::nodes::sources::sigrok_file_source::definition::SigrokFileSource,
            _,
        >(move |state, _inputs, _outputs| {
            crate::nodes::sources::sigrok_file_source::definition::update_source_metadata(
                state,
                source_factory.as_ref(),
            );
        });
    })
}

/// Binds Sigrok decoder catalog discovery to one editor registry.
pub fn sigrok_decoder_editor_override(
    scanner: Arc<dyn SigrokCatalogScanner>,
) -> GraphNodeEditorOverride {
    GraphNodeEditorOverride::new(SIGROK_DECODER_ID, move |registry| {
        let scanner = Arc::clone(&scanner);
        registry.register_with_state_update::<
            crate::nodes::decoders::sigrok_decoder::definition::SigrokDecoderDefinition,
            _,
        >(move |state, _inputs, _outputs| {
            crate::nodes::decoders::sigrok_decoder::definition::update_catalog(
                state,
                scanner.as_ref(),
            );
        });
    })
}

/// Returns the U3Pro16 builder override for one host-selected source factory.
pub fn u3pro16_capability_override(
    source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sources::dslogic_u3pro16::builder::capability_override(source_factory)
}

/// Returns the DSL file-source override for one host acquisition factory.
///
/// # Parameters
/// - `source_factory`: Input consumed by this operation.
pub fn dsl_file_source_capability_override(
    source_factory: Arc<dyn DslFileSourceFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sources::file_source::builder::capability_override(source_factory)
}

/// Returns the Sigrok file-source override for one host acquisition factory.
pub fn sigrok_file_source_capability_override(
    source_factory: Arc<dyn SigrokFileSourceFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sources::sigrok_file_source::builder::capability_override(source_factory)
}

/// Returns the binary-file sink override for one host destination factory.
pub fn binary_file_writer_capability_override(
    writer_factory: Arc<dyn BinaryFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sinks::file_writer::builder::capability_override(writer_factory)
}

/// Returns the CSV sink override for one host destination factory. The text
/// factory backs the pre-formatted-lines mode (a `TextSample` stream on the
/// `Data` input is written verbatim).
pub fn csv_word_writer_capability_override(
    writer_factory: Arc<dyn CsvWordWriterFactory>,
    text_writer_factory: Arc<dyn TextFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sinks::csv_writer::builder::capability_override(
        writer_factory,
        text_writer_factory,
    )
}

/// Returns the text-file sink override for one host destination factory.
pub fn text_file_writer_capability_override(
    writer_factory: Arc<dyn TextFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sinks::text_file_writer::builder::capability_override(writer_factory)
}

/// Returns the Sigrok decoder builder override for one host runtime.
pub fn sigrok_decoder_capability_override(
    runtime: Arc<dyn SigrokDecoderRuntime>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::decoders::sigrok_decoder::builder::capability_override(runtime)
}

/// Builds graph-node templates from portable Sigrok discovery metadata.
pub fn sigrok_node_templates(
    snapshot: &SigrokCatalogSnapshot,
) -> Vec<node_graph::api::NodeTemplate> {
    crate::nodes::decoders::sigrok_decoder::definition::node_templates(snapshot)
}

#[cfg(test)]
mod host_configuration_tests {
    use std::sync::Arc;

    use logic_analyzer_capture_formats::CaptureSourceConstructionError;
    use logic_analyzer_capture_formats::dsl_file::{DslFileSourceConfig, DslFileSourceFactory};
    use node_graph::api::{GraphDocumentBuilder, NodeDef, NodeTypeRegistry};
    use platform_artifacts::ArtifactRepository;
    use platform_runtime::WorkExecutor;
    use signal_capture_session::{
        CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle,
        CaptureSourceMetadata, CaptureSourceMetadataError, CaptureSourcePresentation,
    };
    use signal_runtime::ProcessNodeConstruction;

    use super::dsl_file_source_editor_override;
    use crate::nodes::sources::file_source::definition::DslFileSource;

    struct NamedMetadata {
        channel: String,
    }

    impl CaptureSourceMetadata for NamedMetadata {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
        }

        fn presentation(
            &self,
        ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
            Ok(None)
        }

        fn cache_identity(&self) -> CaptureSourceCacheIdentity {
            CaptureSourceCacheIdentity::Dynamic
        }

        fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
            Ok(Some(vec![self.channel.clone()]))
        }
    }

    struct NamedDslFactory {
        channel: String,
    }

    impl DslFileSourceFactory for NamedDslFactory {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
        }

        fn metadata(&self, _config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
            Arc::new(NamedMetadata {
                channel: self.channel.clone(),
            })
        }

        fn create(
            &self,
            _name: &str,
            _config: DslFileSourceConfig,
            _artifact_repository: Arc<dyn ArtifactRepository>,
            _work_executor: Arc<dyn WorkExecutor>,
        ) -> Result<
            ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
            CaptureSourceConstructionError,
        > {
            Err(CaptureSourceConstructionError::diagnostic(
                "not used by editor metadata tests",
            ))
        }
    }

    fn document_with_channel(channel: &str) -> GraphDocumentBuilder {
        let source_factory: Arc<dyn DslFileSourceFactory> = Arc::new(NamedDslFactory {
            channel: channel.to_owned(),
        });
        let mut registry = NodeTypeRegistry::new();
        dsl_file_source_editor_override(source_factory).apply(&mut registry);
        let mut document = GraphDocumentBuilder::new(registry);
        let node = document.add_node(DslFileSource::name()).unwrap();
        let mut state = DslFileSource::state();
        state.file.value = "same-capture.dsl".into();
        assert!(document.set_node_state(node, serde_json::to_value(state).unwrap()));
        document
    }

    #[test]
    fn editor_registries_keep_host_metadata_factories_instance_owned() {
        let first = document_with_channel("First host channel");
        let second = document_with_channel("Second host channel");

        let first_outputs = &first.graph().nodes.values().next().unwrap().outputs;
        let second_outputs = &second.graph().nodes.values().next().unwrap().outputs;
        assert_eq!(first_outputs[0].name, "First host channel");
        assert_eq!(second_outputs[0].name, "Second host channel");
    }
}
