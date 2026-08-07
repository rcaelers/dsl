//! Built-in LogicConduit graph-node feature bundle.
//!
//! This crate owns concrete node definitions, builders, registrations, saved-state
//! migrations, payloads, socket styling, and presentation metadata. It contributes
//! those features through the graph registry and graph capabilities without making generic compiler, runtime,
//! viewer, or widget code depend on node names, ports, or protocols.

#[cfg(test)]
mod architecture_tests;
mod host_configuration;
mod link;
mod nodes;
mod payloads;
mod presentation;
mod sockets;
#[cfg(test)]
mod test_support;

pub use host_configuration::{
    binary_file_writer_capability_override, csv_word_writer_capability_override,
    dsl_file_source_capability_override, dsl_file_source_editor_override,
    sigrok_decoder_capability_override, sigrok_decoder_editor_override,
    sigrok_file_source_capability_override, sigrok_file_source_editor_override,
    sigrok_node_templates, text_file_writer_capability_override, u3pro16_capability_override,
};
pub use link::link;
pub use payloads::ProtocolPacketLaneSnapshot;
