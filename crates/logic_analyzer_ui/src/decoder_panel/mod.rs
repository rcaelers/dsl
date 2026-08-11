//! Decoder-table panel state, caching, interaction, and rendering.
//!
//! **Owned data and invariants.** `DecoderPanels` keeps each panel's source, format, column
//! visibility, popup focus, and loaded table cache coherent.
//!
//! **Facade.** Siblings use `DecoderPanels`, its persisted state, and the decoder-table model
//! re-exported here.
//!
//! **Permitted owner dependencies.** The owner consumes generic derived-lane queries,
//! node-supplied table metadata, viewer presentation primitives, and egui.
//!
//! **Explicit exclusions.** It does not decode protocols, execute graph nodes, own derived
//! storage, arrange application panels, or infer behavior from decoder names.

mod model;
mod panel;

pub(crate) use model::{DecoderTableColumn, DecoderTableRegistry, DecoderTableSource};
pub(crate) use panel::{DecoderPanels, DecoderPanelsState};
