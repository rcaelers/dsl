//! Owner of graph-service access and the foreground graph-run lifecycle.
//!
//! The owner keeps the active run, its semantic baseline, preview revision, edit debounce,
//! revision-tagged preparation task, progress clock, run status, overlay candidates, and
//! cache-clear task coherent. It does not own graph documents, graph widgets, capture sessions,
//! presentation catalogs, or user notifications.

mod state;

pub(crate) use state::{GraphRunLifecycle, GraphRunPoll};
