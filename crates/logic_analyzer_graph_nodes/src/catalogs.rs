use logic_analyzer_graph_api::node::DirectoryNodeCatalog;

use crate::nodes::decoders::sigrok_decoder::catalog::SigrokDirectoryCatalog;

pub fn native_node_catalogs() -> Vec<Box<dyn DirectoryNodeCatalog>> {
    let settings_path = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("logic-conduit")
        .join("sigrok_decoders.json");
    vec![Box::new(SigrokDirectoryCatalog::new(settings_path))]
}
