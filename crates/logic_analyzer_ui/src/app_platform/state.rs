use std::collections::HashSet;
use std::path::PathBuf;

use node_graph::NodeId;

pub(crate) enum FileCommand {
    New,
    Load,
    LoadPath(PathBuf),
    ClearRecent,
    Save,
    SaveAs,
    SaveCaptureData,
    Quit,
}

pub(crate) enum GuardedAction {
    Quit,
    New,
    LoadPath(PathBuf),
}

const MAX_RECENT_FILES: usize = 10;

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedState {
    #[serde(flatten)]
    ui: super::PersistedUiState,
    recent_files: Vec<PathBuf>,
}

fn normalize_recent_files(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            result.push(path);
        }
        if result.len() >= MAX_RECENT_FILES {
            break;
        }
    }
    result
}

pub(crate) struct PlatformState {
    pub(crate) current_file: Option<PathBuf>,
    pub(crate) saved_graph: serde_json::Value,
    pub(crate) pending_guarded_action: Option<GuardedAction>,
    pub(crate) allow_close: bool,
    pub(crate) recent_files: Vec<PathBuf>,
    pub(crate) confirm_clear_recent: bool,
    pub(crate) confirm_clear_derived_caches: bool,
    pub(crate) derived_cache_nodes: HashSet<NodeId>,
    pub(crate) capture_presentation_identity: Option<String>,
}

impl PlatformState {
    pub(crate) fn restore(
        cc: &eframe::CreationContext,
        widget: &mut node_graph::NodeGraphWidget,
    ) -> Self {
        let persisted = cc
            .storage
            .and_then(|storage| eframe::get_value::<PersistedState>(storage, eframe::APP_KEY));
        if let Some(state) = persisted.as_ref() {
            state.ui.clone().restore(widget);
        }
        let recent_files = persisted
            .map(|state| normalize_recent_files(state.recent_files))
            .unwrap_or_default();
        let saved_graph = widget
            .snapshot_value()
            .expect("new graph should always serialize");
        Self {
            current_file: None,
            saved_graph,
            pending_guarded_action: None,
            allow_close: false,
            recent_files,
            confirm_clear_recent: false,
            confirm_clear_derived_caches: false,
            derived_cache_nodes: HashSet::new(),
            capture_presentation_identity: None,
        }
    }

    pub(crate) fn recent_files(&self) -> &[PathBuf] {
        &self.recent_files
    }

    pub(crate) fn push_recent_file(&mut self, path: PathBuf) {
        let mut paths = self.recent_files.clone();
        paths.insert(0, path);
        self.recent_files = normalize_recent_files(paths);
    }

    pub(crate) fn save(
        &self,
        storage: &mut dyn eframe::Storage,
        graph_ui_prefs: node_graph::GraphUiPrefs,
    ) {
        let state = PersistedState {
            ui: super::PersistedUiState::capture(graph_ui_prefs),
            recent_files: self.recent_files.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &state);
    }
}

#[cfg(test)]
mod tests {
    use super::PersistedState;

    #[test]
    fn legacy_panel_layout_fields_are_ignored() {
        let legacy = serde_json::json!({
            "analyzer_split": 0.37,
            "graph_ui_prefs": {
                "panel_width": 320.0,
                "panel_tab": null,
                "minimap_visible": true,
            },
            "recent_files": [],
        });
        let restored: PersistedState = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.ui.graph_ui_prefs.panel_width, 320.0);
    }
}
