//! Target-independent derived-data cache planning over an injected repository.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::CaptureCacheIdentity;
use node_graph::api::{GraphState, NodeId};
use signal_processing::{ArtifactRepository, PersistentStoreConfig, WorkExecutor};

use super::OutputSubscriptionPlan;
use super::derived_cache_backend::{
    DerivedCacheBackend, DerivedCacheLookup, RepositoryDerivedCacheBackend,
};
use super::errors::CompileError;
use super::graph::{BuilderRegistry, CompiledEdge, CompiledGraph, compiled_node};
const DERIVED_CACHE_ABI_VERSION: u32 = 2;

/// Result of removing persistent derived-data cache entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DerivedCacheClearStats {
    pub removed_entries: usize,
    pub removed_bytes: u64,
}

/// Diagnostics for one validated persistent derived-data cache entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedCacheEntrySnapshot {
    pub total_bytes: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub item_count: u64,
    pub index_item_count: u64,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
}

pub(crate) fn clear_entry(
    config: &PersistentStoreConfig,
) -> Result<DerivedCacheClearStats, String> {
    signal_processing::derived_word_store::clear_cache_entry(config)
        .map(|stats| DerivedCacheClearStats {
            removed_entries: stats.removed_entries,
            removed_bytes: stats.removed_bytes,
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn clear_repository(
    repository: &Arc<dyn ArtifactRepository>,
) -> Result<DerivedCacheClearStats, String> {
    signal_processing::derived_word_store::clear_cache(repository)
        .map(|stats| DerivedCacheClearStats {
            removed_entries: stats.removed_entries,
            removed_bytes: stats.removed_bytes,
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn inspect_entry(
    config: &PersistentStoreConfig,
) -> Result<Option<DerivedCacheEntrySnapshot>, String> {
    signal_processing::derived_word_store::inspect_cache_entry(config)
        .map(|entry| {
            entry.map(|entry| DerivedCacheEntrySnapshot {
                total_bytes: entry.total_bytes,
                data_bytes: entry.data_bytes,
                index_bytes: entry.index_bytes,
                item_count: entry.word_count,
                index_item_count: entry.block_count as u64,
                first_timestamp_ns: entry.first_timestamp_ns,
                last_timestamp_ns: entry.last_timestamp_ns,
            })
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn assign_derived_word_caches(compiled: &mut CompiledGraph, registry: &BuilderRegistry) {
    let collector_ids: Vec<_> = compiled
        .nodes
        .iter()
        .filter(|node| node.data_collector)
        .map(|node| node.id)
        .collect();
    let mut assignments = Vec::new();
    for collector_id in collector_ids {
        let member_count = compiled_node(compiled, collector_id)
            .resolved
            .member_count(0);
        let mut caches = vec![None; member_count];
        for (member, slot) in caches.iter_mut().enumerate() {
            let input_name = format!("in{member}");
            let Some(edge) = compiled.edges.iter().find(|edge| {
                edge.to.0 == collector_id
                    && edge.to.1 == input_name
                    && registry.payload_uses_persistent_cache(edge.kind)
            }) else {
                continue;
            };
            if let Some(key) = persistent_lane_key(compiled, collector_id, member, edge) {
                *slot = Some(PersistentStoreConfig::new(key));
            }
        }
        assignments.push((collector_id, caches));
    }
    for (collector_id, caches) in assignments {
        let node = compiled
            .nodes
            .iter_mut()
            .find(|node| node.id == collector_id)
            .expect("data collector exists");
        node.derived_word_caches = caches;
    }
}

pub(crate) fn configure_repository(
    compiled: &mut CompiledGraph,
    repository: &Arc<dyn ArtifactRepository>,
) {
    for config in compiled
        .nodes
        .iter_mut()
        .flat_map(|node| node.derived_word_caches.iter_mut().flatten())
    {
        config.artifact_repository = Arc::clone(repository);
    }
}

pub(crate) fn prepare_execution(
    compiled: &CompiledGraph,
    registry: &BuilderRegistry,
) -> (CompiledGraph, bool) {
    prepare_execution_with_backend(compiled, registry, &RepositoryDerivedCacheBackend)
}

pub(crate) fn prepare_execution_with_backend(
    compiled: &CompiledGraph,
    registry: &BuilderRegistry,
    backend: &dyn DerivedCacheBackend,
) -> (CompiledGraph, bool) {
    let mut execution = compiled.clone();
    let mut cached_inputs = HashSet::new();
    for collector in &compiled.nodes {
        if !collector.data_collector {
            continue;
        }
        for (member, config) in collector.derived_word_caches.iter().enumerate() {
            let Some(config) = config else {
                continue;
            };
            if backend.lookup(config) == DerivedCacheLookup::Hit {
                cached_inputs.insert((collector.id, format!("in{member}")));
            }
        }
    }
    if cached_inputs.is_empty() {
        return (execution, false);
    }
    execution
        .edges
        .retain(|edge| !cached_inputs.contains(&(edge.to.0, edge.to.1.clone())));

    let mut reachable: HashSet<_> = execution
        .nodes
        .iter()
        .filter(|node| {
            node.data_collector
                || registry
                    .get(&node.builder)
                    .is_some_and(RuntimeBuilder::is_sink)
        })
        .map(|node| node.id)
        .collect();
    let mut stack: Vec<_> = reachable.iter().copied().collect();
    while let Some(node_id) = stack.pop() {
        for edge in execution.edges.iter().filter(|edge| edge.to.0 == node_id) {
            if reachable.insert(edge.from.0) {
                stack.push(edge.from.0);
            }
        }
    }
    execution.nodes.retain(|node| reachable.contains(&node.id));
    execution
        .edges
        .retain(|edge| reachable.contains(&edge.from.0) && reachable.contains(&edge.to.0));
    (execution, true)
}

pub(crate) fn prepare_cached_preview(compiled: &CompiledGraph) -> Option<CompiledGraph> {
    prepare_cached_preview_with_backend(compiled, &RepositoryDerivedCacheBackend)
}

pub(crate) fn prepare_cached_preview_with_backend(
    compiled: &CompiledGraph,
    backend: &dyn DerivedCacheBackend,
) -> Option<CompiledGraph> {
    let mut preview = compiled.clone();
    let mut any_hit = false;
    preview.nodes.retain_mut(|node| {
        if !node.data_collector {
            return false;
        }
        let retained = node
            .resolved
            .members(0)
            .into_iter()
            .filter_map(|(member, input)| {
                let config = node.derived_word_caches.get(member)?.as_ref()?;
                (backend.lookup(config) == DerivedCacheLookup::Hit)
                    .then(|| (input.clone(), config.clone()))
            })
            .collect::<Vec<_>>();
        if retained.is_empty() {
            return false;
        }
        let mut resolved = logic_analyzer_graph_api::node_support::ResolvedInputs::default();
        node.derived_word_caches.clear();
        for (member, (input, config)) in retained.into_iter().enumerate() {
            resolved.insert(0, member, input);
            node.derived_word_caches.push(Some(config));
        }
        node.resolved = resolved;
        any_hit = true;
        true
    });
    preview.edges.clear();
    any_hit.then_some(preview)
}

pub(crate) fn schedule_maintenance(
    compiled: &CompiledGraph,
    work_executor: &Arc<dyn WorkExecutor>,
) {
    if !work_executor.supports_long_running_tasks() {
        return;
    }
    let configs = compiled
        .nodes
        .iter()
        .flat_map(|node| node.derived_word_caches.iter().flatten())
        .collect::<Vec<_>>();
    let Some(first) = configs.first() else {
        return;
    };
    let repository = Arc::clone(&first.artifact_repository);
    let max_total_bytes = first.max_cache_bytes;
    let pinned_keys = configs
        .iter()
        .map(|config| config.cache_key)
        .collect::<Vec<_>>();
    let submitted = work_executor.submit(Box::new(move || {
        let _ = signal_processing::derived_word_store::cleanup_cache(
            &repository,
            max_total_bytes,
            &pinned_keys,
        );
    }));
    drop(submitted);
}

pub(crate) fn cache_configs_by_node(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    repository: &Arc<dyn ArtifactRepository>,
) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
    let mut compiled = super::graph::lower_with_subscriptions(graph, registry, subscriptions)?;
    configure_repository(&mut compiled, repository);
    let mut result: HashMap<NodeId, Vec<PersistentStoreConfig>> = HashMap::new();
    for collector in compiled.nodes.iter().filter(|node| node.data_collector) {
        for (member, config) in collector.derived_word_caches.iter().enumerate() {
            let Some(config) = config else {
                continue;
            };
            let input_name = format!("in{member}");
            let Some(edge) = compiled
                .edges
                .iter()
                .find(|edge| edge.to.0 == collector.id && edge.to.1 == input_name)
            else {
                continue;
            };

            let mut stack = vec![collector.id, edge.from.0];
            let mut visited = HashSet::new();
            while let Some(node_id) = stack.pop() {
                if !visited.insert(node_id) {
                    continue;
                }
                let configs = result.entry(node_id).or_default();
                if !configs
                    .iter()
                    .any(|existing| existing.cache_key == config.cache_key)
                {
                    configs.push(config.clone());
                }
                stack.extend(
                    compiled
                        .edges
                        .iter()
                        .filter(|incoming| incoming.to.0 == node_id)
                        .map(|incoming| incoming.from.0),
                );
            }
        }
    }
    Ok(result)
}

pub(crate) fn persistent_lane_key(
    compiled: &CompiledGraph,
    collector_id: NodeId,
    member: usize,
    edge: &CompiledEdge,
) -> Option<[u8; 32]> {
    let mut memo = HashMap::new();
    let upstream = persistent_upstream_key(compiled, edge.from.0, &mut memo)?;
    let collector = compiled_node(compiled, collector_id);
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, b"dsl-derived-lane-cache-v1");
    hash_field(&mut hasher, env!("CARGO_PKG_VERSION").as_bytes());
    hash_field(&mut hasher, &DERIVED_CACHE_ABI_VERSION.to_le_bytes());
    hash_field(&mut hasher, &canonical_json_bytes(&collector.state));
    hash_field(&mut hasher, &(member as u64).to_le_bytes());
    hash_field(&mut hasher, edge.from.1.as_bytes());
    hash_field(&mut hasher, edge.kind.name().as_bytes());
    hash_field(&mut hasher, &upstream);
    Some(*hasher.finalize().as_bytes())
}

fn persistent_upstream_key(
    compiled: &CompiledGraph,
    node_id: NodeId,
    memo: &mut HashMap<NodeId, [u8; 32]>,
) -> Option<[u8; 32]> {
    if let Some(key) = memo.get(&node_id) {
        return Some(*key);
    }
    let node = compiled_node(compiled, node_id);
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, b"node");
    hash_field(&mut hasher, node.builder.as_bytes());
    hash_field(&mut hasher, &canonical_json_bytes(&node.state));
    match node.capture_cache_identity {
        CaptureCacheIdentity::NotCapture => {}
        CaptureCacheIdentity::Dynamic => return None,
        CaptureCacheIdentity::Stable(identity) => hash_field(&mut hasher, &identity),
    }
    let mut incoming: Vec<_> = compiled
        .edges
        .iter()
        .filter(|edge| edge.to.0 == node_id)
        .collect();
    incoming.sort_by(|left, right| {
        (&left.to.1, &left.from.1, left.kind.name()).cmp(&(
            &right.to.1,
            &right.from.1,
            right.kind.name(),
        ))
    });
    for edge in incoming {
        hash_field(&mut hasher, edge.to.1.as_bytes());
        hash_field(&mut hasher, edge.from.1.as_bytes());
        hash_field(&mut hasher, edge.kind.name().as_bytes());
        hash_field(
            &mut hasher,
            &persistent_upstream_key(compiled, edge.from.0, memo)?,
        );
    }
    let key = *hasher.finalize().as_bytes();
    memo.insert(node_id, key);
    Some(key)
}

fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    fn append(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null => output.push(b'n'),
            Value::Bool(value) => output.extend_from_slice(if *value { b"t" } else { b"f" }),
            Value::Number(value) => {
                output.push(b'#');
                append_bytes(output, value.to_string().as_bytes());
            }
            Value::String(value) => {
                output.push(b'"');
                append_bytes(output, value.as_bytes());
            }
            Value::Array(values) => {
                output.push(b'[');
                for value in values {
                    append(value, output);
                }
                output.push(b']');
            }
            Value::Object(values) => {
                output.push(b'{');
                let mut fields: Vec<_> = values.iter().collect();
                fields.sort_by_key(|(name, _)| *name);
                for (name, value) in fields {
                    append_bytes(output, name.as_bytes());
                    append(value, output);
                }
                output.push(b'}');
            }
        }
    }
    fn append_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
        output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        output.extend_from_slice(bytes);
    }
    let mut output = Vec::new();
    append(value, &mut output);
    output
}

fn hash_field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod cache_policy_tests {
    use std::sync::Arc;

    use signal_processing::{
        ArtifactRepository, IndexedAnnotationWriter, LiveStoreConfig, MemoryArtifactRepository,
        PersistentStoreConfig, Word,
    };

    use super::{clear_entry, clear_repository, inspect_entry};

    #[test]
    fn injected_memory_repository_supports_the_complete_cache_lifecycle() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let persistent = PersistentStoreConfig::new([0x5a; 32])
            .with_artifact_repository(Arc::clone(&repository));
        let config = LiveStoreConfig {
            persistence: Some(persistent.clone()),
            ..LiveStoreConfig::default()
        };
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer
            .append_batch(&[Word::spanning(0x42, 100, 20)])
            .unwrap();
        writer.finish().unwrap();

        let snapshot = inspect_entry(&persistent)
            .unwrap()
            .expect("the published memory-backed cache must be discoverable");
        assert_eq!(snapshot.item_count, 1);
        assert_eq!(snapshot.first_timestamp_ns, Some(100));

        drop((writer, store));
        let cleared = clear_entry(&persistent).unwrap();
        assert_eq!(cleared.removed_entries, 1);
        assert!(cleared.removed_bytes > 0);
        assert_eq!(inspect_entry(&persistent).unwrap(), None);
        assert_eq!(clear_repository(&repository).unwrap().removed_entries, 0);
    }
}
