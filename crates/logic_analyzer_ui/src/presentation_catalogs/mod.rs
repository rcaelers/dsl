//! Owner of application presentation catalogs and their persisted viewer selections.
//!
//! The owner retains derived lanes, output/table subscriptions, decoder/plugin panel bindings,
//! sampling-overlay selections, and viewer row order as one synchronized presentation snapshot. It
//! does not execute graphs, query capture storage, or render application frames.

mod error;
mod state;

pub(crate) use error::PresentationBindingError;
pub(crate) use state::PresentationCatalogs;
