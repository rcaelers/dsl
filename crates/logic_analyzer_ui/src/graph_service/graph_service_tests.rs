use std::sync::Arc;

use logic_analyzer_graph_plan::OutputSubscriptionPlan;
use logic_analyzer_graph_runtime::{DerivedCacheClearStats, InlineSourcePreparationExecutor};
use node_graph::api::GraphState;
use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository};
use platform_runtime::InlineWorkExecutor;
use signal_derived::DecodedBlockCacheHandle;
use signal_runtime::CooperativeAppManagerFactory;

use super::graph_compiler::{UiGraphService, graph_service_with_execution};
use crate::live_capture::capture_availability;

fn configured_service() -> UiGraphService {
    let mut service = graph_service_with_execution(
        Box::new(InlineSourcePreparationExecutor),
        Arc::new(CooperativeAppManagerFactory),
        Arc::new(InlineWorkExecutor),
    );
    let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
    service.set_artifact_repository(repository);
    service.set_decoded_block_cache(DecodedBlockCacheHandle::default());
    service
}

#[test]
fn concrete_service_uses_the_injected_repository_and_runtime() {
    let mut service = configured_service();
    service.set_output_subscriptions(OutputSubscriptionPlan::default());

    let mut clear = service.start_clear_derived_caches().unwrap();
    let stats = clear
        .poll(16)
        .expect("the empty in-memory repository clears in one cooperative poll")
        .unwrap();
    assert_eq!(stats, DerivedCacheClearStats::default());

    let errors = service
        .derived_cache_configs_by_node(&GraphState::default())
        .unwrap_err();
    assert_eq!(errors[0].message, "Graph has no sink (add a File Writer)");
}

#[test]
fn capture_discovery_uses_the_concrete_graph_service() {
    let service = configured_service();

    let availability = capture_availability(&GraphState::default(), &service, None);

    assert_eq!(
        availability.reason(),
        Some("The graph has no live capture source")
    );
}
