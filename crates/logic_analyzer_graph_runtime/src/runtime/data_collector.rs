//! Presentation-neutral collection of retained derived data.

use logic_analyzer_graph_capabilities::node_support::{NodeBuildContext, ResolvedInputs};
use logic_analyzer_graph_plan::ProcessingPayloadCatalog;
use signal_derived::{CollectedLaneRequest, DerivedDataCollector, LiveStoreConfig};
use signal_runtime::ProcessNode;

pub(crate) struct DataCollectorBuilder;

impl DataCollectorBuilder {
    pub(crate) fn build_with_lane_names(
        name: &str,
        resolved: &ResolvedInputs,
        lane_names: &[(usize, String)],
        payload_catalog: &dyn ProcessingPayloadCatalog,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let mut collector = DerivedDataCollector::new()
            .with_name(name)
            .with_retention(ctx.derived_data_retention());
        for (member, lane_name) in lane_names {
            let input = resolved
                .get(0, *member)
                .ok_or_else(|| format!("collector input {member} is unresolved"))?;
            let descriptor = payload_catalog
                .payloads()
                .descriptor_by_type_id(input.kind.type_id())
                .ok_or_else(|| format!("collector cannot retain {:?}", input.kind))?
                .clone();
            let mut request = CollectedLaneRequest::new(
                lane_name,
                *member,
                ctx.derived_lanes().clone(),
                descriptor,
                ctx.derived_data_retention(),
            )
            .with_decoded_block_cache(ctx.decoded_block_cache());
            if let Some(persistent) = ctx.derived_word_cache(*member) {
                request = request.with_indexed_store(
                    LiveStoreConfig {
                        persistence: Some(persistent.clone()),
                        ..LiveStoreConfig::default()
                    }
                    .with_work_executor(ctx.work_executor())
                    .with_artifact_repository(ctx.artifact_repository()),
                );
            }
            let (request, diagnostic_name) = payload_catalog
                .configure_collected_lane_request(input.kind, request, *member, input, ctx)?;
            let adapter = payload_catalog
                .payloads()
                .adapter_by_type_id(input.kind.type_id())
                .ok_or_else(|| {
                    format!(
                        "payload '{}' ({}) has no ingestion adapter",
                        diagnostic_name,
                        request.payload().stable_id()
                    )
                })?;
            let ingestor = adapter.create_ingestor(request).map_err(|error| {
                format!(
                    "collector adapter for '{}' could not create '{}': {error}",
                    diagnostic_name, lane_name
                )
            })?;
            collector = collector.with_ingestor(ingestor);
        }
        Ok(Box::new(collector))
    }
}
