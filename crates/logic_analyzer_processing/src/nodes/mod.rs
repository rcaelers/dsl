//! # `logic_analyzer_processing::nodes`
//!
//! ## Responsibility
//!
//! This namespace groups all supported concrete UI-independent processing nodes by their operational
//! role: decoders, logic processors, sinks, and sources.
//!
//! ## Child owners
//!
//! - [decoders](nodes/decoders.md)
//! - [logic](nodes/logic.md)
//! - [sinks](nodes/sinks.md)
//! - [sources](nodes/sources.md)
//!
//! ## Boundaries
//!
//! It does not define graph editor nodes, select host implementations, or coordinate a run. Each child
//! module owns one concrete processing behavior and exposes only its supported configuration and
//! factory contract.

//! Concrete processing nodes used by logic-analyzer graphs.
//!
//! The child namespaces group supported UI-independent behavior by operational
//! role. They do not define graph-editor nodes, select host implementations, or
//! coordinate graph execution.

pub mod decoders;
pub mod logic;
pub mod sinks;
pub mod sources;
