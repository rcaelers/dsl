//! Viewer presentation for parallel-decoder output.

use logic_analyzer_graph_capabilities::node_support::{
    DecoderTableCellMode, DecoderTableColumnDescriptor,
};
use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerLaneRendererRegistration};

pub(crate) fn parallel_table_column(def_index: usize) -> Option<DecoderTableColumnDescriptor> {
    (def_index == 0).then(|| {
        DecoderTableColumnDescriptor::new(
            "decoder",
            "data",
            "Data",
            0,
            true,
            DecoderTableCellMode::Single,
            "primary",
            PARALLEL_TABLE_RENDERER,
        )
    })
}

const PARALLEL_TABLE_RENDERER: &str = "org.logicconduit.renderer.parallel-table/v1";

inventory::submit! {
    ViewerLaneRendererRegistration::new(PARALLEL_TABLE_RENDERER, || {
        std::sync::Arc::new(DefaultViewerLaneRenderer)
    })
}

#[cfg(test)]
mod presentation_tests {
    use super::*;

    #[test]
    fn words_are_an_explicit_table_source() {
        assert!(parallel_table_column(1).is_none());
        let table = parallel_table_column(0).unwrap();
        assert_eq!(table.source_key, "decoder");
        assert!(table.row_anchor);
    }
}
