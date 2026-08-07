//! Owner of timeline-marker discovery bindings and synchronization diagnostics.
//!
//! The owner maps viewer marker identities to graph-node marker identities and suppresses repeated
//! discovery diagnostics. It does not discover or edit graph nodes, render markers, or persist
//! cursor state; `App` composes those operations at the graph-service and viewer boundaries.

mod state;

pub(crate) use state::TimelineMarkerBindings;
