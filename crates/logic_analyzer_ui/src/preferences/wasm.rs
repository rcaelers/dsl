use logic_analyzer_graph_api::node::DirectoryNodeCatalog;

pub(crate) struct PreferencesWindow;

impl PreferencesWindow {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn show(
        &mut self,
        _ctx: &egui::Context,
        _catalogs: &mut [Box<dyn DirectoryNodeCatalog>],
    ) {
    }
}
