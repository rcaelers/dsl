use std::collections::HashSet;
use std::path::{Path, PathBuf};

use node_graph::api::NodeId;

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
    current_file: Option<PathBuf>,
    saved_graph: serde_json::Value,
    pending_guarded_action: Option<GuardedAction>,
    allow_close: bool,
    recent_files: Vec<PathBuf>,
    confirm_clear_recent: bool,
    confirm_clear_derived_caches: bool,
    derived_cache_nodes: HashSet<NodeId>,
}

impl PlatformState {
    pub(crate) fn restore(
        cc: &eframe::CreationContext,
        widget: &mut node_graph::NodeGraphWidget,
        viewer: &mut logic_analyzer_viewer::LogicAnalyzerViewer,
    ) -> Self {
        let persisted = cc
            .storage
            .and_then(|storage| eframe::get_value::<PersistedState>(storage, eframe::APP_KEY));
        if let Some(state) = persisted.as_ref() {
            state.ui.clone().restore(widget, viewer);
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
        }
    }

    pub(crate) fn recent_files(&self) -> &[PathBuf] {
        &self.recent_files
    }

    pub(crate) fn current_file(&self) -> Option<&Path> {
        self.current_file.as_deref()
    }

    pub(crate) fn set_current_file(&mut self, path: PathBuf) {
        self.current_file = Some(path);
    }

    pub(crate) fn clear_current_file(&mut self) {
        self.current_file = None;
    }

    pub(crate) fn mark_saved_graph(&mut self, graph: serde_json::Value) {
        self.saved_graph = graph;
    }

    pub(crate) fn is_saved_graph(&self, graph: &serde_json::Value) -> bool {
        &self.saved_graph == graph
    }

    pub(crate) fn guarded_action(&self) -> Option<&GuardedAction> {
        self.pending_guarded_action.as_ref()
    }

    pub(crate) fn request_guarded_action(&mut self, action: GuardedAction) {
        self.pending_guarded_action = Some(action);
    }

    pub(crate) fn cancel_guarded_action(&mut self) {
        self.pending_guarded_action = None;
    }

    pub(crate) fn take_guarded_action(&mut self) -> Option<GuardedAction> {
        self.pending_guarded_action.take()
    }

    pub(crate) fn close_allowed(&self) -> bool {
        self.allow_close
    }

    pub(crate) fn allow_close(&mut self) {
        self.allow_close = true;
    }

    pub(crate) fn push_recent_file(&mut self, path: PathBuf) {
        let mut paths = self.recent_files.clone();
        paths.insert(0, path);
        self.recent_files = normalize_recent_files(paths);
    }

    pub(crate) fn clear_recent_files(&mut self) {
        self.recent_files.clear();
    }

    pub(crate) fn request_clear_recent_confirmation(&mut self) {
        self.confirm_clear_recent = true;
    }

    pub(crate) fn clear_recent_confirmation_requested(&self) -> bool {
        self.confirm_clear_recent
    }

    pub(crate) fn finish_clear_recent_confirmation(&mut self) {
        self.confirm_clear_recent = false;
    }

    pub(crate) fn request_clear_derived_caches_confirmation(&mut self) {
        self.confirm_clear_derived_caches = true;
    }

    pub(crate) fn clear_derived_caches_confirmation_requested(&self) -> bool {
        self.confirm_clear_derived_caches
    }

    pub(crate) fn finish_clear_derived_caches_confirmation(&mut self) {
        self.confirm_clear_derived_caches = false;
    }

    pub(crate) fn derived_cache_nodes(&self) -> &HashSet<NodeId> {
        &self.derived_cache_nodes
    }

    pub(crate) fn set_derived_cache_nodes(&mut self, nodes: HashSet<NodeId>) {
        self.derived_cache_nodes = nodes;
    }

    pub(crate) fn clear_derived_cache_nodes(&mut self) {
        self.derived_cache_nodes.clear();
    }

    pub(crate) fn save(
        &self,
        storage: &mut dyn eframe::Storage,
        graph_ui_prefs: node_graph::GraphUiPrefs,
        viewer_ui_prefs: logic_analyzer_viewer::ViewerUiPrefs,
    ) {
        let state = PersistedState {
            ui: super::PersistedUiState::capture(graph_ui_prefs, viewer_ui_prefs),
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
        // Saved before the viewer toggles existed: both restore enabled.
        assert!(restored.ui.viewer_measurements_enabled);
        assert!(restored.ui.viewer_snapping_enabled);
    }

    #[test]
    fn viewer_toggles_survive_a_save_and_restore_round_trip() {
        let state = PersistedState {
            ui: super::super::PersistedUiState::capture(
                node_graph::GraphUiPrefs {
                    panel_width: 320.0,
                    panel_tab: None,
                    minimap_visible: true,
                },
                logic_analyzer_viewer::ViewerUiPrefs {
                    measurements_enabled: false,
                    snapping_enabled: true,
                },
            ),
            recent_files: Vec::new(),
        };

        let restored: PersistedState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();

        assert!(!restored.ui.viewer_measurements_enabled);
        assert!(restored.ui.viewer_snapping_enabled);
    }
}
