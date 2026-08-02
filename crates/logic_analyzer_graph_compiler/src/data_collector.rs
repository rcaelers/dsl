//! Presentation-neutral collection of retained derived data.

use std::collections::HashMap;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{NodeBuildContext, PortKind, ResolvedInputs};
use node_graph::api::Socket;
use signal_processing::{CollectedLaneRequest, DerivedDataCollector, LiveStoreConfig, ProcessNode};

use super::graph::BuilderRegistry;

pub(crate) const BUILDER_NAME: &str = "Derived Data Collector";
pub(crate) const OUTPUT_SUBSCRIPTION_BUILDER_NAME: &str = "Output Subscription Collector";

pub(crate) struct DataCollectorBuilder {
    output_subscription: bool,
}

impl DataCollectorBuilder {
    pub(crate) const fn retained_data() -> Self {
        Self {
            output_subscription: false,
        }
    }

    pub(crate) const fn output_subscription() -> Self {
        Self {
            output_subscription: true,
        }
    }

    pub(crate) fn build_with_lane_names(
        name: &str,
        resolved: &ResolvedInputs,
        lane_names: &[(usize, String)],
        registry: &BuilderRegistry,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let mut collector = DerivedDataCollector::new()
            .with_name(name)
            .with_retention(ctx.derived_data_retention());
        for (member, lane_name) in lane_names {
            let input = resolved
                .get(0, *member)
                .ok_or_else(|| format!("collector input {member} is unresolved"))?;
            let descriptor = registry
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
            );
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
            let (request, diagnostic_name) = registry
                .configure_collected_lane_request(input.kind, request, *member, input, ctx)?;
            let adapter = registry
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

    fn default_lane_names(resolved: &ResolvedInputs) -> Vec<(usize, String)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        resolved
            .members(0)
            .into_iter()
            .map(|(member, input)| {
                let count = counts.entry(input.source.clone()).or_default();
                *count += 1;
                let name = if *count == 1 {
                    input.source.clone()
                } else {
                    format!("{} ({count})", input.source)
                };
                (member, name)
            })
            .collect()
    }
}

impl RuntimeBuilder for DataCollectorBuilder {
    fn is_sink(&self) -> bool {
        true
    }

    fn is_data_collector(&self) -> bool {
        true
    }

    fn is_data_subscription(&self) -> bool {
        self.output_subscription
    }

    fn collected_lane_names(
        &self,
        _state: &Value,
        resolved: &ResolvedInputs,
    ) -> Vec<(usize, String)> {
        Self::default_lane_names(resolved)
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn input_port(
        &self,
        _socket: &Socket,
        member_index: usize,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some(format!("in{member_index}"))
    }

    fn output_port(&self, _socket: &Socket, _state: &Value, _kind: PortKind) -> Option<String> {
        None
    }

    fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
        false
    }

    fn build(
        &self,
        _name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Err("data collectors must be materialized through the payload registry".to_owned())
    }
}
