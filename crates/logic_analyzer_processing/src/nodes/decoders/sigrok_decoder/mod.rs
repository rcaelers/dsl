//! Sigrok Python decoder runtime and collected output contracts.

#[cfg(not(target_arch = "wasm32"))]
mod implementation;

#[cfg(not(target_arch = "wasm32"))]
pub use implementation::{
    SigrokChannel, SigrokDecoder, SigrokDecoderConfig, SigrokInitialPin, SigrokOptionValue,
};
