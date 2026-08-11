//! # `buffer`
//!
//! ## Responsibility
//!
//! This module owns the explicit stream-buffer processing node used to decouple one downstream branch.
//!
//! ## Boundaries
//!
//! It does not select the graph-edge capacity policy globally or own scheduler behavior. Its configured
//! capacity applies only to its own runtime input/output boundary.

//! Stream buffer processing node.
//!
//! It implements runtime buffering only; graph configuration and host scheduling
//! remain outside this concrete processing behavior.

mod relay;

pub use relay::BufferNode;
