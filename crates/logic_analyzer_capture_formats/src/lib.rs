//! Portable readers, index builders, and replay sources for logic-analyzer capture formats.
//!
//! This crate owns DSL and Sigrok archive semantics after a host supplies a prepared byte source.
//! File dialogs, native paths, browser handles, graph definitions, and viewer policy remain
//! outside this owner.

pub mod dsl_file;
pub mod sigrok_file;
mod support;
