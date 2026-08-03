use serde::{Deserialize, Serialize};

/// Stable identity of one node in a persisted graph document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

/// Address of one input or output socket on a graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SocketId {
    /// Node that owns the socket.
    pub node: NodeId,
    /// Definition index of the socket on its owner.
    pub index: usize,
    /// Whether the address identifies an input or output socket.
    pub direction: SocketDirection,
}

/// Direction of a graph socket relative to its owning node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SocketDirection {
    /// Socket receives a connection from an upstream output.
    Input,
    /// Socket produces a connection to a downstream input.
    Output,
}
