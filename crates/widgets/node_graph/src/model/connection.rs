use serde::{Deserialize, Serialize};

use super::ids::SocketId;

/// Directed connection from one output socket to one input socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    /// Upstream output socket.
    pub from: SocketId,
    /// Downstream input socket.
    pub to: SocketId,
}
