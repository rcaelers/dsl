use std::collections::HashMap;
use std::path::Path;

use logic_analyzer_graph_compiler::{CompileError, GraphCompiler};
use node_graph::{GraphState, NodeId};
use signal_processing::PersistentStoreConfig;

use super::platform_contract::PlatformGraphService;

impl PlatformGraphService for GraphCompiler {
    fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
        directory: &Path,
    ) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
        GraphCompiler::derived_cache_configs_by_node(self, graph, directory)
    }
}
