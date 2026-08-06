//! Node-editor registration integration for Logic Conduit graph features.
//!
//! This crate is the UI-side counterpart of `logic_analyzer_graph_registry`: plug-ins register
//! widget definitions here under the same stable feature IDs used by their headless capabilities.

mod editor_override;
mod editor_registration;

pub use editor_override::GraphNodeEditorOverride;
pub use editor_registration::{
    GraphNodeEditorRegistration, graph_node_editor_registrations, node_name,
};
