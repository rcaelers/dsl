//! Built-in retained-payload capabilities and presentations.

mod digital;
mod number;
#[cfg(test)]
mod presentation_tests;
mod protocol_packet;
mod protocol_packet_presentation;
mod protocol_packet_retention;
mod text;
mod trigger;
mod word;

#[cfg(test)]
pub(crate) use protocol_packet_presentation::protocol_packet_display;
pub(crate) use protocol_packet_presentation::{
    ProtocolPacketDisplay, ProtocolPacketPresentationRegistration, protocol_packet_fallback_label,
};
pub use protocol_packet_retention::ProtocolPacketLaneSnapshot;
pub(crate) use word::WordSnapshotRenderer;
