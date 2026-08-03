//! # `sigrok_decoder`
//!
//! ## Responsibility
//!
//! This module owns portable Sigrok decoder configuration, execution behavior, and output contracts.
//!
//! ## Boundaries
//!
//! Python-host discovery, interpreter setup, package locations, and concrete execution factories are
//! platform concerns injected through its contracts. The module does not own graph-node presentation.

//! Sigrok Python decoder runtime and discovery contracts.
//!
//! This module owns concrete decoder adaptation and diagnostics. Discovery UI,
//! node controls, and host catalog selection remain outside the processing node.

mod contracts;
mod implementation;

pub use contracts::{
    InitialPin, LogicChunk, MetadataRegistration, MetadataType, OutputRegistration,
    SigrokAnnotationClassDescriptor, SigrokAnnotationRowDescriptor, SigrokCatalogDiagnostic,
    SigrokCatalogDiagnosticKind, SigrokCatalogEntry, SigrokCatalogSnapshot,
    SigrokDecoderChannelDescriptor, SigrokDecoderDescriptor, SigrokDecoderOptionDescriptor,
    SigrokExecution, SigrokExecutionConfig, SigrokExecutionFactory, SigrokExecutionInput,
    SigrokExecutionOptionValue, SigrokExecutionOutput, SigrokOutputKind, SigrokScalarValue,
};
pub use implementation::{
    SigrokChannel, SigrokDecoder, SigrokDecoderConfig, SigrokInitialPin, SigrokOptionValue,
};
