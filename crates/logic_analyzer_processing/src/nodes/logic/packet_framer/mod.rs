//! Protocol-neutral word packet framing.

mod implementation;

pub use implementation::{GatePolarity, PACKET_FRAME_PROTOCOL_ID, PacketFramer};
