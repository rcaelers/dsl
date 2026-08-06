//! UI-independent protocol decoding state machines and host contracts.
//!
//! Graph definitions, socket presentation, renderer metadata, and host runtime selection remain
//! outside this crate.

pub mod i2c_decoder;
pub mod parallel_decoder;
pub mod sigrok_decoder;
pub mod spi_decoder;
pub mod types;
pub mod uart_decoder;
