//! # `logic_analyzer_graph_api::node`
//!
//! ## Responsibility
//!
//! This namespace owns traits and inventory registrations implemented by graph-node and payload
//! plugins, including runtime builders, graph-node registrations, payload registrations, and capture
//! feature factories.
//!
//! ## Boundaries
//!
//! It is an extension contract, not a compiler, node bundle, UI service, or host adapter. Implementers
//! use `node_support` for all supporting values and do not depend on concrete compiler internals. Its
//! current `DirectoryNodeCatalog` path configuration is the documented host-path exception scheduled
//! to move behind UI and platform ownership.

//! Contracts implemented or submitted by graph nodes and compile-time plugins.
//!
//! This namespace owns inventory registrations and the capability-specific feature
//! contracts that plugins implement. It intentionally contains no compiler policy,
//! built-in-node behavior, host paths, or UI operations.

mod catalog;
mod contracts;
mod graph_registration;
mod payload_registration;
mod protocol_packet_presentation;

pub use catalog::{DirectoryNodeCatalog, NodeCatalogStatus};
pub use contracts::{
    CaptureGraphSourceFactory, LiveCaptureFeature, RuntimeBuilder, RuntimeBuilderOverride,
};
pub use graph_registration::{GraphNodeRegistration, graph_node_registrations};
pub use payload_registration::{PayloadRegistration, PayloadRequestConfigurator};
pub use protocol_packet_presentation::{
    ProtocolPacketDisplay, ProtocolPacketPresentationRegistration, protocol_packet_display,
};
