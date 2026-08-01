use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use signal_processing::PersistentStoreConfig;

use super::contract::{CacheClearStats, CacheEntrySnapshot, HostService, OpenDialog, SaveDialog};

struct FakeHostService {
    open_paths: VecDeque<Option<PathBuf>>,
    save_paths: VecDeque<Option<PathBuf>>,
    directories: VecDeque<Option<PathBuf>>,
    saved_graphs: Vec<(PathBuf, serde_json::Value)>,
    cache_result: Result<CacheClearStats, String>,
}

impl Default for FakeHostService {
    fn default() -> Self {
        Self {
            open_paths: VecDeque::new(),
            save_paths: VecDeque::new(),
            directories: VecDeque::new(),
            saved_graphs: Vec::new(),
            cache_result: Ok(CacheClearStats::default()),
        }
    }
}

impl HostService for FakeHostService {
    fn choose_open_file(&mut self, _request: OpenDialog<'_>) -> Option<PathBuf> {
        self.open_paths.pop_front().flatten()
    }

    fn choose_save_file(&mut self, _request: SaveDialog<'_>) -> Option<PathBuf> {
        self.save_paths.pop_front().flatten()
    }

    fn choose_directory(&mut self) -> Option<PathBuf> {
        self.directories.pop_front().flatten()
    }

    fn load_graph(&mut self, _path: &Path) -> Result<node_graph::GraphState, String> {
        Ok(node_graph::GraphState::default())
    }

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String> {
        self.saved_graphs.push((path.to_owned(), graph.clone()));
        Ok(())
    }

    fn clear_cache_entry(
        &mut self,
        _config: &PersistentStoreConfig,
    ) -> Result<CacheClearStats, String> {
        self.cache_result.clone()
    }

    fn clear_cache(&mut self, _directory: &Path) -> Result<CacheClearStats, String> {
        self.cache_result.clone()
    }

    fn inspect_cache_entry(
        &self,
        _config: &PersistentStoreConfig,
    ) -> Result<Option<CacheEntrySnapshot>, String> {
        Ok(None)
    }
}

#[test]
fn fake_host_effects_are_ordered_and_do_not_touch_the_host() {
    let mut host = FakeHostService {
        open_paths: VecDeque::from([Some(PathBuf::from("first.json")), None]),
        save_paths: VecDeque::from([Some(PathBuf::from("saved.json"))]),
        directories: VecDeque::from([Some(PathBuf::from("decoders"))]),
        cache_result: Ok(CacheClearStats {
            removed_entries: 2,
            removed_bytes: 4096,
        }),
        ..FakeHostService::default()
    };

    let open = OpenDialog {
        title: "Open",
        filter_label: "JSON",
        extensions: &["json"],
        initial_directory: None,
    };
    assert_eq!(
        host.choose_open_file(open),
        Some(PathBuf::from("first.json"))
    );
    assert_eq!(host.choose_directory(), Some(PathBuf::from("decoders")));
    assert_eq!(
        host.choose_save_file(SaveDialog {
            title: "Save",
            default_file_name: "pipeline.json",
            filter_label: "JSON",
            extensions: &["json"],
            initial_directory: None,
        }),
        Some(PathBuf::from("saved.json"))
    );
    assert_eq!(
        host.choose_open_file(OpenDialog {
            title: "Open",
            filter_label: "JSON",
            extensions: &["json"],
            initial_directory: None,
        }),
        None,
        "a cancelled dialog is a deterministic non-error result"
    );

    let graph = serde_json::json!({"nodes": []});
    host.save_graph(Path::new("saved.json"), &graph).unwrap();
    assert_eq!(
        host.saved_graphs,
        vec![(PathBuf::from("saved.json"), graph)]
    );
    assert_eq!(
        host.clear_cache(Path::new("cache")).unwrap(),
        CacheClearStats {
            removed_entries: 2,
            removed_bytes: 4096,
        }
    );
}

#[test]
fn fake_host_propagates_effect_failures() {
    let mut host = FakeHostService {
        cache_result: Err("cache is busy".into()),
        ..FakeHostService::default()
    };

    assert_eq!(
        host.clear_cache(Path::new("cache")),
        Err("cache is busy".into())
    );
}
