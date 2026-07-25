use std::fmt;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use logic_analyzer_graph_api::node_support::DecoderTableCellMode;
use logic_analyzer_viewer::{DerivedLaneId, ViewerLaneRenderer, ViewerLaneTrackId};

/// A decoder-table column bound to a collected derived lane.
#[derive(Clone)]
pub(crate) struct DecoderTableColumn {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) lane: DerivedLaneId,
    pub(crate) track: ViewerLaneTrackId,
    pub(crate) row_anchor: bool,
    pub(crate) cell_mode: DecoderTableCellMode,
    pub(crate) renderer: Arc<dyn ViewerLaneRenderer>,
}

impl fmt::Debug for DecoderTableColumn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecoderTableColumn")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("lane", &self.lane)
            .field("track", &self.track)
            .field("row_anchor", &self.row_anchor)
            .field("cell_mode", &self.cell_mode)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DecoderTableSource {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) columns: Vec<DecoderTableColumn>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DecoderTableRegistry {
    inner: Arc<RwLock<Vec<DecoderTableSource>>>,
}

impl DecoderTableRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(&self, source: DecoderTableSource) {
        let mut sources = self.inner.write().unwrap();
        if let Some(existing) = sources.iter_mut().find(|existing| existing.id == source.id) {
            *existing = source;
        } else {
            sources.push(source);
        }
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<'_, Vec<DecoderTableSource>> {
        self.inner.read().unwrap()
    }
}
