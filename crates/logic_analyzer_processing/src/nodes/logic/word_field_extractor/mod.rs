//! # `word_field_extractor`
//!
//! ## Responsibility
//!
//! This module owns configured field extraction from runtime word values.
//!
//! ## Boundaries
//!
//! It does not own editor controls, display formatting, or protocol-specific interpretation beyond its
//! declared word-field configuration.

//! Word bit-field extraction processing node.
//!
//! It extracts configured fields from generic words without owning any concrete
//! decoder, graph socket definition, or display formatting policy.

mod implementation;

pub use implementation::WordFieldExtractor;
