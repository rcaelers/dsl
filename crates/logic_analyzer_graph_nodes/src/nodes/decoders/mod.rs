//! Concrete protocol-decoder graph nodes.

mod binary_decoder;
mod i2c_decoder;
mod parallel_decoder;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod sigrok_decoder;
mod spi_decoder;
mod uart_decoder;
