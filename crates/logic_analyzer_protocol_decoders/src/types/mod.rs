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
//! source/decoder behavior. Values with wider generic meaning remain in `signal_capture_session`.

//! Protocol-neutral value conventions shared by processing nodes.
//!
//! These conventions are shared by concrete processing nodes without imposing
//! graph, UI, transport, or presentation behavior.

mod digital;

pub use digital::{BitOrder, CsPolarity, Endianness};
