//! # `i2c_decoder`
//!
//! ## Responsibility
//!
//! This module owns I²C signal decoding and I²C packet production from its configured processing
//! inputs.
//!
//! ## Boundaries
//!
//! It does not own I²C graph sockets, packet rendering, saved-node migration, or UI panels; those
//! belong to the matching graph-node feature.

//! Native I²C protocol decoder.
//!
//! It turns generic sampled inputs into I²C words and diagnostics. Socket layout,
//! controls, and presentation are owned by the corresponding graph-node feature.

mod decoder;

pub use decoder::{I2C_PROTOCOL_ID, I2cDecoder};
