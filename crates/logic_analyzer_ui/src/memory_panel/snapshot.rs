use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    CacheMemorySnapshot, MemoryServiceSnapshot, PersistentCacheSnapshot,
    PersistentCacheSnapshotState,
};
use crate::app::App;

impl App {
    pub(crate) fn cache_memory_snapshot(&mut self) -> CacheMemorySnapshot {
        let decoded = self.host_service.decoded_block_cache_snapshot();
        let mut snapshot = CacheMemorySnapshot {
            services: vec![decoded.map_or_else(
                || MemoryServiceSnapshot {
                    name: "Decoded block cache".to_owned(),
                    state: "Unavailable".to_owned(),
                    detail: "The data-plane adapter does not provide this cache".to_owned(),
                    used_bytes: None,
                    budget_bytes: None,
                },
                |decoded| {
                    MemoryServiceSnapshot {
                        name: "Decoded block cache".to_owned(),
                        state: if decoded.entries == 0 {
                            "Empty"
                        } else {
                            "Ready"
                        }
                        .to_owned(),
                        detail: format!(
                            "{} block(s) · {} hit(s) · {} miss(es)",
                            decoded.entries, decoded.hits, decoded.misses
                        ),
                        used_bytes: Some(decoded.memory_bytes as u64),
                        budget_bytes: Some(decoded.budget_bytes as u64),
                    }
                },
            )],
            persistent_caches: Vec::new(),
        };
        let inventory = match self
            .graph_service
            .derived_cache_configs_by_node(self.node_graph.graph())
        {
            Ok(inventory) => inventory,
            Err(errors) => {
                snapshot.services.push(MemoryServiceSnapshot {
                    name: "Persistent derived cache".to_owned(),
                    state: "Unavailable".to_owned(),
                    detail: errors.first().map_or_else(
                        || "Graph cannot be lowered".to_owned(),
                        |error| error.message.clone(),
                    ),
                    used_bytes: None,
                    budget_bytes: None,
                });
                return snapshot;
            }
        };
        let mut entries = BTreeMap::new();
        for (node_id, configs) in inventory {
            let owner = self
                .node_graph
                .graph()
                .nodes
                .get(&node_id)
                .map(|node| node.title.clone());
            for config in configs {
                let (_, owners): &mut (signal_processing::PersistentStoreConfig, BTreeSet<String>) =
                    entries
                        .entry(config.cache_key)
                        .or_insert_with(|| (config.clone(), BTreeSet::new()));
                if let Some(owner) = &owner {
                    owners.insert(owner.clone());
                }
            }
        }
        for (_, (config, owners)) in entries {
            let inspected = self.graph_service.inspect_derived_cache_entry(&config);
            let (state, info) = match inspected {
                Ok(Some(info)) => (PersistentCacheSnapshotState::Ready, Some(info)),
                Ok(None) => (PersistentCacheSnapshotState::Missing, None),
                Err(error) => (PersistentCacheSnapshotState::Unreadable(error), None),
            };
            snapshot.persistent_caches.push(PersistentCacheSnapshot {
                cache_key: config.cache_key,
                owners: owners.into_iter().collect(),
                repository: repository_description(config.artifact_repository.capabilities()),
                state,
                total_bytes: info.map(|info| info.total_bytes),
                data_bytes: info.map(|info| info.data_bytes),
                index_bytes: info.map(|info| info.index_bytes),
                items: info.map(|info| info.item_count),
                index_items: info.map(|info| info.index_item_count),
            });
        }
        let ready = snapshot
            .persistent_caches
            .iter()
            .filter(|entry| entry.state == PersistentCacheSnapshotState::Ready)
            .count();
        let bytes = snapshot
            .persistent_caches
            .iter()
            .filter_map(|entry| entry.total_bytes)
            .fold(0u64, u64::saturating_add);
        snapshot.services.push(MemoryServiceSnapshot {
            name: "Persistent derived cache".to_owned(),
            state: if snapshot.persistent_caches.is_empty() {
                "Empty"
            } else if ready == 0 {
                "Missing"
            } else {
                "Ready"
            }
            .to_owned(),
            detail: format!(
                "{ready} ready of {} selected graph entr{}",
                snapshot.persistent_caches.len(),
                if snapshot.persistent_caches.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ),
            used_bytes: Some(bytes),
            budget_bytes: None,
        });
        snapshot
    }
}

fn repository_description(capabilities: signal_artifacts::RepositoryCapabilities) -> String {
    match (capabilities.durable, capabilities.immutable_regions) {
        (true, true) => "Durable mapped storage",
        (true, false) => "Durable storage",
        (false, _) => "Process memory",
    }
    .to_owned()
}
