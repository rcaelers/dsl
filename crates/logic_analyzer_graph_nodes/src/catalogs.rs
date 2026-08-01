use std::sync::Arc;

use logic_analyzer_graph_api::node::DirectoryNodeCatalog;
use signal_processing::WorkExecutor;

use crate::nodes::decoders::sigrok_decoder::catalog::SigrokDirectoryCatalog;

pub fn native_node_catalogs(
    work_executor: Arc<dyn WorkExecutor>,
) -> Vec<Box<dyn DirectoryNodeCatalog>> {
    let settings_path = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("logic-conduit")
        .join("sigrok_decoders.json");
    vec![Box::new(SigrokDirectoryCatalog::with_work_executor(
        settings_path,
        work_executor,
    ))]
}
