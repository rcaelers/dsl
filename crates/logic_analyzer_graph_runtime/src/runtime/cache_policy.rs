//! Target-independent derived-data cache planning over an injected repository.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use logic_analyzer_graph_capabilities::node_support::CaptureCacheIdentity;
use logic_analyzer_graph_plan::{
    ProcessingEdge, ProcessingGraph, ProcessingNode, SamplingOverlayCandidate,
};
use node_graph::api::NodeId;
use signal_artifacts::ArtifactRepository;
use signal_derived::PersistentStoreConfig;
use signal_derived::derived_word_store::PersistentCacheClearTask;
use signal_runtime::{WorkExecutor, WorkTask};

use super::derived_cache_backend::{
    DerivedCacheBackend, DerivedCacheLookup, RepositoryDerivedCacheBackend,
};
const DERIVED_CACHE_ABI_VERSION: u32 = 2;

/// Result of removing persistent derived-data cache entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DerivedCacheClearStats {
    /// Number of persistent cache entries removed.
    pub removed_entries: usize,
    /// Total artifact bytes removed.
    pub removed_bytes: u64,
}

/// Diagnostics for one validated persistent derived-data cache entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedCacheEntrySnapshot {
    /// Combined size of data and index artifacts.
    pub total_bytes: u64,
    /// Bytes occupied by retained data artifacts.
    pub data_bytes: u64,
    /// Bytes occupied by index artifacts.
    pub index_bytes: u64,
    /// Number of retained data items.
    pub item_count: u64,
    /// Number of index entries.
    pub index_item_count: u64,
    /// Timestamp of the earliest retained item, if known.
    pub first_timestamp_ns: Option<u64>,
    /// Timestamp of the latest retained item, if known.
    pub last_timestamp_ns: Option<u64>,
}

/// Host-scheduled removal of all persistent derived-data artifacts.
pub struct DerivedCacheClearTask {
    mode: DerivedCacheClearMode,
}

enum DerivedCacheClearMode {
    Background {
        work: Box<dyn WorkTask>,
        result: Arc<Mutex<Option<Result<DerivedCacheClearStats, String>>>>,
    },
    Cooperative(PersistentCacheClearTask),
}

impl DerivedCacheClearTask {
    /// Advances cooperative cleanup by a bounded number of repository
    /// artifacts, or polls host-threaded cleanup without blocking.
    ///
    /// # Parameters
    /// - `artifact_budget`: Maximum number of artifacts to remove cooperatively.
    pub fn poll(
        &mut self,
        artifact_budget: usize,
    ) -> Option<Result<DerivedCacheClearStats, String>> {
        match &mut self.mode {
            DerivedCacheClearMode::Background { work, result } => {
                if !work.is_finished() {
                    return None;
                }
                match result.lock() {
                    Ok(mut result) => result.take().or_else(|| {
                        Some(Err("derived-data cache worker returned no result".into()))
                    }),
                    Err(_) => Some(Err("derived-data cache worker result was poisoned".into())),
                }
            }
            DerivedCacheClearMode::Cooperative(task) => match task.advance(artifact_budget) {
                Ok(Some(stats)) => Some(Ok(DerivedCacheClearStats {
                    removed_entries: stats.removed_entries,
                    removed_bytes: stats.removed_bytes,
                })),
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            },
        }
    }
}

pub(crate) fn clear_entry(
    config: &PersistentStoreConfig,
) -> Result<DerivedCacheClearStats, String> {
    signal_derived::derived_word_store::clear_cache_entry(config)
        .map(|stats| DerivedCacheClearStats {
            removed_entries: stats.removed_entries,
            removed_bytes: stats.removed_bytes,
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn clear_repository(
    repository: &Arc<dyn ArtifactRepository>,
) -> Result<DerivedCacheClearStats, String> {
    signal_derived::derived_word_store::clear_cache(repository)
        .map(|stats| DerivedCacheClearStats {
            removed_entries: stats.removed_entries,
            removed_bytes: stats.removed_bytes,
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn start_clear_repository(
    repository: &Arc<dyn ArtifactRepository>,
    work_executor: &Arc<dyn WorkExecutor>,
) -> Result<DerivedCacheClearTask, String> {
    if !work_executor.supports_long_running_tasks() {
        return PersistentCacheClearTask::new(Arc::clone(repository))
            .map(|task| DerivedCacheClearTask {
                mode: DerivedCacheClearMode::Cooperative(task),
            })
            .map_err(|error| error.to_string());
    }

    let result = Arc::new(Mutex::new(None));
    let worker_result = Arc::clone(&result);
    let worker_repository = Arc::clone(repository);
    let work = work_executor.submit(Box::new(move || {
        let cleared = clear_repository(&worker_repository);
        if let Ok(mut result) = worker_result.lock() {
            *result = Some(cleared);
        }
    }))?;
    Ok(DerivedCacheClearTask {
        mode: DerivedCacheClearMode::Background { work, result },
    })
}

pub(crate) fn inspect_entry(
    config: &PersistentStoreConfig,
) -> Result<Option<DerivedCacheEntrySnapshot>, String> {
    signal_derived::derived_word_store::inspect_cache_entry(config)
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

pub(crate) fn assign_derived_word_caches(compiled: &mut ProcessingGraph) {
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
                    && compiled.payload_catalog.uses_persistent_cache(edge.kind)
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

pub(crate) fn assign_sampling_point_caches(compiled: &mut ProcessingGraph) {
    let assignments = compiled
        .sampling_overlays
        .iter()
        .map(|candidate| {
            (
                candidate.node_id(),
                (!candidate.uses_retained_word_lane())
                    .then(|| persistent_sampling_point_key(compiled, candidate.node_id()))
                    .flatten(),
            )
        })
        .collect::<Vec<_>>();
    for candidate in &mut compiled.sampling_overlays {
        let cache_key = assignments
            .iter()
            .find_map(|(node_id, key)| (*node_id == candidate.node_id()).then_some(*key))
            .flatten();
        candidate.set_cache_key(cache_key);
    }
}

pub(crate) fn configure_repository(
    compiled: &mut ProcessingGraph,
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

pub(crate) fn prepare_execution(compiled: &ProcessingGraph) -> (ProcessingGraph, bool) {
    prepare_execution_with_backend(compiled, &RepositoryDerivedCacheBackend)
}

pub(crate) fn prepare_execution_with_backend(
    compiled: &ProcessingGraph,
    backend: &dyn DerivedCacheBackend,
) -> (ProcessingGraph, bool) {
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
        .filter(|node| node.data_collector || node.sink)
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

pub(crate) fn prepare_cached_preview(compiled: &ProcessingGraph) -> Option<ProcessingGraph> {
    prepare_cached_preview_with_backend(compiled, &RepositoryDerivedCacheBackend)
}

pub(crate) fn prepare_cached_preview_with_backend(
    compiled: &ProcessingGraph,
    backend: &dyn DerivedCacheBackend,
) -> Option<ProcessingGraph> {
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
                    .then(|| (member, input.clone(), config.clone()))
            })
            .collect::<Vec<_>>();
        if retained.is_empty() {
            return false;
        }
        let mut resolved =
            logic_analyzer_graph_capabilities::node_support::ResolvedInputs::default();
        let mut collected_lane_names = Vec::new();
        let mut collected_source_labels = Vec::new();
        node.derived_word_caches.clear();
        for (member, (original_member, input, config)) in retained.into_iter().enumerate() {
            resolved.insert(0, member, input);
            node.derived_word_caches.push(Some(config));
            if let Some((_, name)) = node
                .collected_lane_names
                .iter()
                .find(|(candidate, _)| *candidate == original_member)
            {
                collected_lane_names.push((member, name.clone()));
            }
            if let Some((_, label)) = node
                .collected_source_labels
                .iter()
                .find(|(candidate, _)| *candidate == original_member)
            {
                collected_source_labels.push((member, label.clone()));
            }
        }
        node.resolved = resolved;
        node.collected_lane_names = collected_lane_names;
        node.collected_source_labels = collected_source_labels;
        any_hit = true;
        true
    });
    preview.edges.clear();
    any_hit.then_some(preview)
}

pub(crate) fn schedule_maintenance(
    compiled: &ProcessingGraph,
    repository: &Arc<dyn ArtifactRepository>,
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
    let first = configs.first();
    let sampling_key = compiled
        .sampling_overlays
        .iter()
        .find_map(SamplingOverlayCandidate::cache_key);
    if first.is_none() && sampling_key.is_none() {
        return;
    }
    let repository = Arc::clone(repository);
    let max_total_bytes = first.map_or_else(
        || {
            let key = sampling_key.expect("a derived or sampling cache exists");
            PersistentStoreConfig::new(key).max_cache_bytes
        },
        |config| config.max_cache_bytes,
    );
    let mut pinned_keys = configs
        .iter()
        .map(|config| config.cache_key)
        .collect::<Vec<_>>();
    pinned_keys.extend(
        compiled
            .sampling_overlays
            .iter()
            .filter_map(SamplingOverlayCandidate::cache_key),
    );
    let submitted = work_executor.submit(Box::new(move || {
        let _ = signal_derived::derived_word_store::cleanup_cache(
            &repository,
            max_total_bytes,
            &pinned_keys,
        );
    }));
    drop(submitted);
}

pub(crate) fn cache_configs_by_node(
    compiled: &ProcessingGraph,
    repository: &Arc<dyn ArtifactRepository>,
) -> HashMap<NodeId, Vec<PersistentStoreConfig>> {
    let mut compiled = compiled.clone();
    assign_derived_word_caches(&mut compiled);
    assign_sampling_point_caches(&mut compiled);
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
    for candidate in &compiled.sampling_overlays {
        let Some(cache_key) = candidate.cache_key() else {
            continue;
        };
        let config =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(repository));
        let mut stack = vec![candidate.node_id()];
        let mut visited = HashSet::new();
        while let Some(node_id) = stack.pop() {
            if !visited.insert(node_id) {
                continue;
            }
            let configs = result.entry(node_id).or_default();
            if !configs
                .iter()
                .any(|existing| existing.cache_key == cache_key)
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
    result
}

pub(crate) fn prepare_sampling_point_stores(
    compiled: &mut ProcessingGraph,
    execution: &ProcessingGraph,
    lanes: &signal_derived::DerivedLanes,
    repository: &Arc<dyn ArtifactRepository>,
    work_executor: &Arc<dyn WorkExecutor>,
) {
    for candidate in &mut compiled.sampling_overlays {
        if candidate.install_retained_word_provider(lanes.clone()) {
            continue;
        }
        let Some(cache_key) = candidate.cache_key() else {
            continue;
        };
        let config =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(repository));
        let executed = execution
            .nodes
            .iter()
            .any(|node| node.id == candidate.node_id());
        let store = if executed {
            signal_derived::SamplingPointStore::create_persistent(config, Arc::clone(work_executor))
                .ok()
        } else {
            signal_derived::SamplingPointStore::open_persistent(&config)
                .ok()
                .flatten()
        };
        if let Some(store) = store {
            candidate.set_points(store);
        }
    }
}

pub(crate) fn open_sampling_point_stores(
    compiled: &mut ProcessingGraph,
    lanes: &signal_derived::DerivedLanes,
    repository: &Arc<dyn ArtifactRepository>,
) -> bool {
    let mut opened = false;
    for candidate in &mut compiled.sampling_overlays {
        if candidate.install_retained_word_provider(lanes.clone()) {
            opened = true;
            continue;
        }
        let Some(cache_key) = candidate.cache_key() else {
            continue;
        };
        let config =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(repository));
        if let Ok(Some(store)) = signal_derived::SamplingPointStore::open_persistent(&config) {
            candidate.set_points(store);
            opened = true;
        }
    }
    opened
}

pub(crate) fn persistent_lane_key(
    compiled: &ProcessingGraph,
    collector_id: NodeId,
    member: usize,
    edge: &ProcessingEdge,
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

fn persistent_sampling_point_key(compiled: &ProcessingGraph, node_id: NodeId) -> Option<[u8; 32]> {
    let mut memo = HashMap::new();
    let upstream = persistent_upstream_key(compiled, node_id, &mut memo)?;
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, b"dsl-sampling-point-cache-v1");
    hash_field(&mut hasher, env!("CARGO_PKG_VERSION").as_bytes());
    hash_field(&mut hasher, &DERIVED_CACHE_ABI_VERSION.to_le_bytes());
    hash_field(&mut hasher, &upstream);
    Some(*hasher.finalize().as_bytes())
}

fn persistent_upstream_key(
    compiled: &ProcessingGraph,
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

fn compiled_node(compiled: &ProcessingGraph, id: NodeId) -> &ProcessingNode {
    compiled
        .nodes
        .iter()
        .find(|node| node.id == id)
        .expect("node in compiled graph")
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use signal_artifacts::{ArtifactRepository, MemoryArtifactRepository};
    use signal_derived::{IndexedAnnotationWriter, LiveStoreConfig, PersistentStoreConfig, Word};
    use signal_runtime::{WorkExecutor, WorkExecutorTask, WorkTask};

    use super::{
        DerivedCacheClearStats, clear_entry, clear_repository, inspect_entry,
        start_clear_repository,
    };

    struct QueuedWorkExecutor {
        queued: Mutex<Option<WorkExecutorTask>>,
        finished: Arc<AtomicBool>,
    }

    impl QueuedWorkExecutor {
        fn new() -> Self {
            Self {
                queued: Mutex::new(None),
                finished: Arc::new(AtomicBool::new(false)),
            }
        }

        fn run_queued(&self) {
            self.queued.lock().unwrap().take().unwrap()();
            self.finished.store(true, Ordering::Release);
        }
    }

    struct QueuedWorkTask(Arc<AtomicBool>);

    impl WorkTask for QueuedWorkTask {
        fn is_finished(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }

        fn wait(self: Box<Self>) {}
    }

    impl WorkExecutor for QueuedWorkExecutor {
        fn available_parallelism(&self) -> usize {
            1
        }

        fn supports_long_running_tasks(&self) -> bool {
            true
        }

        fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
            *self.queued.lock().unwrap() = Some(task);
            Ok(Box::new(QueuedWorkTask(Arc::clone(&self.finished))))
        }
    }

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

    #[test]
    fn threaded_cache_clear_is_only_polled_by_the_caller() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let queued = Arc::new(QueuedWorkExecutor::new());
        let executor: Arc<dyn WorkExecutor> = queued.clone();

        let mut task = start_clear_repository(&repository, &executor).unwrap();
        assert!(task.poll(1).is_none());

        queued.run_queued();
        assert_eq!(task.poll(1), Some(Ok(DerivedCacheClearStats::default())));
    }
}
