//! Portable persisted graph-document records and graph-editing invariants.
//!
//! This crate owns graph identities, nodes, sockets, connections, frames, neutral presentation
//! values, and saved-document serialization. It has no widget, compiler, runtime, or host
//! dependency.

mod connection;
mod frame;
mod graph;
mod ids;
mod node;
mod presentation;
mod socket;

pub use connection::Connection;
pub use frame::{Frame, FrameId};
pub use graph::{GraphMetadata, GraphState};
pub use ids::{NodeId, SocketDirection, SocketId};
pub use node::{BadgeSeverity, Node, NodeBadge, NodeKind, NodeMetadata};
pub use presentation::{GraphColor, GraphPosition};
pub use socket::{Socket, SocketReference, SocketShape, VariadicInfo};
