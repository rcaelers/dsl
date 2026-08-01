//! Wasm persistent-cache capability boundary.
//!
//! The derived-word store remains available through its in-memory backend;
//! only filesystem persistence and cache-pruning are absent.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use node_graph::api::{GraphState, NodeId};
use signal_processing::{ArtifactRepository, PersistentStoreConfig};

use super::OutputSubscriptionPlan;
use super::errors::CompileError;
use super::graph::{BuilderRegistry, CompiledGraph};

pub(crate) fn assign_derived_word_caches(compiled: &mut CompiledGraph, registry: &BuilderRegistry) {
    // Keep cache policy part of the common subscription contract even though
    // this platform has no persistent-cache implementation.
    let _ = compiled
        .edges
        .iter()
        .any(|edge| registry.payload_uses_persistent_cache(edge.kind));
}

pub(crate) fn configure_directory(_compiled: &mut CompiledGraph, _directory: Option<&Path>) {}

pub(crate) fn configure_repository(
    _compiled: &mut CompiledGraph,
    _repository: &Arc<dyn ArtifactRepository>,
) {
}

pub(crate) fn prepare_execution(
    compiled: &CompiledGraph,
    _registry: &BuilderRegistry,
) -> (CompiledGraph, bool) {
    (compiled.clone(), false)
}

pub(crate) fn prepare_cached_preview(_compiled: &CompiledGraph) -> Option<CompiledGraph> {
    None
}

pub(crate) fn cache_configs_by_node(
    _graph: &GraphState,
    _registry: &BuilderRegistry,
    _subscriptions: &OutputSubscriptionPlan,
    _directory: &std::path::Path,
    _repository: &Arc<dyn ArtifactRepository>,
) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
    Ok(HashMap::new())
}
