use serde_json::Value;

use node_graph_document::GraphState;

#[test]
fn checked_in_graph_keeps_its_serialized_document_shape() {
    let source = include_str!("../graphs/spi_decode_pipeline.json");
    let original = serde_json::from_str::<Value>(source).expect("fixture is valid JSON");
    let document = serde_json::from_value::<GraphState>(original.clone())
        .expect("fixture remains a valid graph document");
    let serialized = serde_json::to_value(document).expect("graph document remains serializable");

    assert_eq!(serialized, original);
}
