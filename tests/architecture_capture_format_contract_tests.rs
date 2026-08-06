use std::sync::Arc;

use logic_analyzer_capture_formats::dsl_file::{
    DslFileSourceConfig, DslFileSourceFactory, unavailable_source_factory,
};
use logic_analyzer_capture_formats::sigrok_file::{
    SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};
use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository};
use platform_runtime::{InlineWorkExecutor, WorkExecutor};
use signal_capture_session::CaptureSourceMetadata;
use signal_runtime::ProcessNodeConstruction;

fn assert_send_sync<T: ?Sized + Send + Sync>() {}

#[test]
fn capture_source_factories_expose_neutral_metadata_bearing_construction_contracts() {
    assert_send_sync::<dyn DslFileSourceFactory>();
    assert_send_sync::<dyn SigrokFileSourceFactory>();

    let artifact_repository: Arc<dyn ArtifactRepository> =
        Arc::new(MemoryArtifactRepository::new());
    let work_executor: Arc<dyn WorkExecutor> = Arc::new(InlineWorkExecutor);

    let dsl_config = DslFileSourceConfig::new("capture.dsl", []);
    let dsl_factory = unavailable_source_factory();
    let _: Arc<dyn CaptureSourceMetadata> = dsl_factory.metadata(dsl_config.clone());
    let dsl_construction: Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> =
        dsl_factory.create(
            "DSL capture",
            dsl_config,
            artifact_repository,
            Arc::clone(&work_executor),
        );
    assert!(dsl_construction.is_err());

    let sigrok_config = SigrokFileSourceConfig::new("demo.sr", [], true);
    let sigrok_factory = portable_source_factory();
    let _: Arc<dyn CaptureSourceMetadata> = sigrok_factory.metadata(sigrok_config.clone());
    let sigrok_construction: Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        String,
    > = sigrok_factory.create("Sigrok demo", sigrok_config, work_executor);
    assert!(sigrok_construction.is_ok());
}
