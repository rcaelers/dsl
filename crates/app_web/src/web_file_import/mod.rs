//! Browser capture-file acquisition and prepared-source adapters.

mod dialog;
mod dsl;
mod error;
mod registry;
mod sigrok;
mod worker_source;

#[cfg(test)]
mod web_file_import_tests;

pub(crate) use dialog::BrowserFilePickerService;
pub(crate) use dsl::dsl_source_factory;
pub(crate) use registry::BrowserFileRegistry;
pub(crate) use sigrok::sigrok_source_factory;
pub(crate) use worker_source::{
    capture_metadata, capture_worker_operations, worker_dsl_file_source_factory,
    worker_sigrok_file_source_factory,
};
