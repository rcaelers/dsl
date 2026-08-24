use logic_analyzer_viewer::ViewerUiPrefs;

/// Restores the enabled default for viewer toggles saved before they existed.
fn enabled() -> bool {
    true
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct PersistedUiState {
    pub(crate) graph_ui_prefs: node_graph::GraphUiPrefs,
    // Stored as plain fields rather than a nested `ViewerUiPrefs` so the
    // viewer widget stays free of a serde dependency.
    #[serde(default = "enabled")]
    pub(crate) viewer_measurements_enabled: bool,
    #[serde(default = "enabled")]
    pub(crate) viewer_snapping_enabled: bool,
}

impl PersistedUiState {
    pub(crate) fn capture(
        graph_ui_prefs: node_graph::GraphUiPrefs,
        viewer_ui_prefs: ViewerUiPrefs,
    ) -> Self {
        Self {
            graph_ui_prefs,
            viewer_measurements_enabled: viewer_ui_prefs.measurements_enabled,
            viewer_snapping_enabled: viewer_ui_prefs.snapping_enabled,
        }
    }

    pub(crate) fn restore(
        self,
        widget: &mut node_graph::NodeGraphWidget,
        viewer: &mut logic_analyzer_viewer::LogicAnalyzerViewer,
    ) {
        let mut prefs = self.graph_ui_prefs;
        prefs.panel_tab = match prefs.panel_tab.as_deref() {
            Some("Node") => Some("node".to_owned()),
            Some("Auxiliary" | "View") => Some("view".to_owned()),
            _ => prefs.panel_tab,
        };
        widget.set_ui_prefs(prefs);
        viewer.set_ui_prefs(ViewerUiPrefs {
            measurements_enabled: self.viewer_measurements_enabled,
            snapping_enabled: self.viewer_snapping_enabled,
        });
    }
}
