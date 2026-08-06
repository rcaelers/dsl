//! # `packet_framer`
//!
//! ## Responsibility
//!
//! This module owns framing configured event or word inputs into protocol-packet runtime values.
//!
//! ## Boundaries
//!
//! It does not decide packet display labels, viewer spans, or protocol renderer registration. The
//! matching graph-node feature owns that presentation metadata.

//! Protocol-neutral word packet framing.
//!
//! It frames generic words without treating any protocol name or presentation as
//! special. Concrete renderers consume the explicit payload metadata it produces.

mod implementation;

pub use implementation::{GatePolarity, PACKET_FRAME_PROTOCOL_ID, PacketFramer};
