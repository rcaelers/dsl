//! # `word_matcher`
//!
//! ## Responsibility
//!
//! This module owns configured matching, holdoff, selection, and event production over word streams.
//!
//! ## Boundaries
//!
//! It does not own graph state editing, trigger-panel policy, or packet presentation.

//! Word-matching processing node.
//!
//! It evaluates configured matches over generic words. Presentation of resulting
//! events and all editor controls stay with separate owners.

mod implementation;

pub use implementation::{MatchOp, MatchTimestamp, PredicateMode, WordMatcher};
