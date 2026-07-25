#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct PersistedUiState {
    pub(crate) graph_ui_prefs: node_graph::GraphUiPrefs,
}

impl PersistedUiState {
    pub(crate) fn capture(graph_ui_prefs: node_graph::GraphUiPrefs) -> Self {
        Self { graph_ui_prefs }
    }

    pub(crate) fn restore(self, widget: &mut node_graph::NodeGraphWidget) {
        let mut prefs = self.graph_ui_prefs;
        prefs.panel_tab = match prefs.panel_tab.as_deref() {
            Some("Node") => Some("node".to_owned()),
            Some("Auxiliary" | "View") => Some("view".to_owned()),
            _ => prefs.panel_tab,
        };
        widget.set_ui_prefs(prefs);
    }
}
