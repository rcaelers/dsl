//! # `logic_analyzer_protocol_decoders::types`
//!
//! ## Responsibility
//!
//! This namespace owns protocol-neutral processing value conventions shared by concrete processing
//! nodes.
//!
//! ## Boundaries
//!
//! It does not own generic runtime payload contracts, graph sockets, widget presentation, or concrete
//! source/decoder behavior. Values with wider generic meaning remain in the generic signal crates.

//! Protocol-neutral value conventions shared by processing nodes.
//!
//! These conventions are shared by concrete processing nodes without imposing
//! graph, UI, transport, or presentation behavior.

mod digital;
mod packet;

pub use digital::{BitOrder, CsPolarity, Endianness};
pub use packet::{ProtocolPacket, ProtocolValue};
