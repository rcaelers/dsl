use serde_json::Value;

use logic_analyzer_graph_api::node::LiveCaptureFeature;

pub(crate) fn feature(_state: &Value) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
    Ok(None)
}
