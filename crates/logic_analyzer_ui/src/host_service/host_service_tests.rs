use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::contract::{HostService, OpenDialog, SaveDialog};

#[derive(Default)]
struct FakeHostService {
    open_paths: VecDeque<Option<PathBuf>>,
    save_paths: VecDeque<Option<PathBuf>>,
    directories: VecDeque<Option<PathBuf>>,
    saved_graphs: Vec<(PathBuf, serde_json::Value)>,
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
}

#[test]
fn fake_host_effects_are_ordered_and_do_not_touch_the_host() {
    let mut host = FakeHostService {
        open_paths: VecDeque::from([Some(PathBuf::from("first.json")), None]),
        save_paths: VecDeque::from([Some(PathBuf::from("saved.json"))]),
        directories: VecDeque::from([Some(PathBuf::from("decoders"))]),
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
}
