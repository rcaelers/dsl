//! Sigrok Python decoder runtime and discovery contracts.

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
