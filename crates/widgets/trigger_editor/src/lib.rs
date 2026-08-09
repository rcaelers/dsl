//! Generic schema-driven editor for provider-neutral trigger programs.
//!
//! The editor renders the trigger schema and emits edits against generic trigger
//! contracts. Device-specific predicate semantics, acquisition behavior, and
//! application composition belong to their respective owners. The crate root
//! is the stable facade over private contract, reducer, presentation, and
//! widget owners.

mod contract;
mod error;
mod model;
mod presentation;
mod widget;

pub use contract::{TriggerEditorAction, TriggerEditorChannel, TriggerEditorResponse};
pub use error::TriggerEditorError;
pub use model::TriggerEditorModel;
pub use widget::TriggerEditor;

#[cfg(test)]
mod trigger_editor_tests;
