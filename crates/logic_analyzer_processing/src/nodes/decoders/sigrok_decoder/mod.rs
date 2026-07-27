//! Sigrok Python decoder runtime and discovery contracts.

#[cfg(not(target_arch = "wasm32"))]
mod implementation;
#[cfg(all(not(target_arch = "wasm32"), feature = "developer-tools"))]
mod upstream_validation;

#[cfg(not(target_arch = "wasm32"))]
pub use implementation::{
    SigrokChannel, SigrokDecoder, SigrokDecoderConfig, SigrokInitialPin, SigrokOptionValue,
};
#[cfg(all(not(target_arch = "wasm32"), feature = "developer-tools"))]
pub use upstream_validation::{validate_spi_chunk_boundaries, validate_spi_oracle};

#[cfg(not(target_arch = "wasm32"))]
pub use crate::support::sigrokdecode::discovery::{
    SigrokAnnotationClassDescriptor, SigrokAnnotationRowDescriptor, SigrokCatalogDiagnostic,
    SigrokCatalogDiagnosticKind, SigrokCatalogEntry, SigrokCatalogSnapshot, SigrokDecoderCatalog,
    SigrokDecoderChannelDescriptor, SigrokDecoderDescriptor, SigrokDecoderOptionDescriptor,
    SigrokOutputKind, SigrokPackageDiscovery, SigrokScalarValue, SigrokSearchPathDiscovery,
    SigrokSearchPathError, discover_sigrok_decoder,
};
