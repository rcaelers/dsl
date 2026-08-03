//! # `spi_decoder`
//!
//! ## Responsibility
//!
//! This module owns SPI stream decoding, transaction assembly, and protocol-packet production.
//!
//! ## Boundaries
//!
//! It does not own SPI graph controls, packet labels, viewer rendering, or saved-state migration.
//! Those protocol-specific presentation concerns remain in the SPI graph-node feature.

//! SPI decoder processing node.
//!
//! It owns SPI signal decoding from generic sampled inputs, not graph sockets,
//! panel controls, protocol tables, or execution lifecycle.

mod implementation;

pub use implementation::{SPI_TRANSACTION_PROTOCOL_ID, SpiDecoder, SpiMode};
