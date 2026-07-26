//! Sigrok Python decoder runtime and discovery contracts.

#[cfg(not(target_arch = "wasm32"))]
mod implementation;

#[cfg(not(target_arch = "wasm32"))]
pub use implementation::{
    SigrokChannel, SigrokDecoder, SigrokDecoderConfig, SigrokInitialPin, SigrokOptionValue,
};

#[cfg(not(target_arch = "wasm32"))]
pub use crate::support::sigrokdecode::discovery::{
    SigrokAnnotationClassDescriptor, SigrokAnnotationRowDescriptor, SigrokCatalogDiagnostic,
    SigrokCatalogDiagnosticKind, SigrokCatalogEntry, SigrokCatalogSnapshot, SigrokDecoderCatalog,
    SigrokDecoderChannelDescriptor, SigrokDecoderDescriptor, SigrokDecoderOptionDescriptor,
    SigrokOutputKind, SigrokScalarValue, discover_sigrok_decoder,
};
