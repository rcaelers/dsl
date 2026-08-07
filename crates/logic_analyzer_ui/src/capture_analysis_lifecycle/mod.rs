//! Owner of capture coordination, trigger discovery, and capture-analysis execution state.
//!
//! The owner keeps acquisition state, its graph snapshot, analysis run, configuration-epoch
//! synchronization, trigger configuration, availability, and storage projection coherent. It does
//! not render controls, edit graph documents, execute foreground runs, or format notifications.

mod state;

pub(crate) use state::CaptureAnalysisLifecycle;
