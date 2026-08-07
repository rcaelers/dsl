//! Graph builder for constructing streaming node graphs.
//!
//! Provides a builder API for connecting nodes with typed channels.

use std::any::TypeId;
use std::collections::HashMap;

use super::errors::ConnectionError;

/// Unique identifier for a node in the graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

impl NodeId {
    /// Wraps a graph-local numeric node identifier.
    ///
    /// # Parameters
    /// - `id`: Graph-local numeric identifier.
    pub fn new(id: usize) -> Self {
        Self(id)
    }

    /// Returns the graph-local numeric identifier.
    pub fn as_usize(&self) -> usize {
        self.0
    }
}

/// Represents a connection between two nodes
#[derive(Debug, Clone)]
pub struct Connection {
    pub from_node: NodeId,
    pub from_port: usize,
    pub to_node: NodeId,
    pub to_port: usize,
    pub buffer_size: usize,
    pub type_id: TypeId,
}

/// Builder for constructing a streaming graph
pub struct GraphBuilder {
    next_node_id: usize,
    nodes: HashMap<NodeId, NodeInfo>,
    connections: Vec<Connection>,
}

struct NodeInfo {
    name: String,
    input_ports: Vec<PortInfo>,
    output_ports: Vec<PortInfo>,
}

#[derive(Clone)]
struct PortInfo {
    type_id: TypeId,
    type_name: String,
}

impl GraphBuilder {
    /// Create a new graph builder
    pub fn new() -> Self {
        Self {
            next_node_id: 0,
            nodes: HashMap::new(),
            connections: Vec::new(),
        }
    }

    /// Add a processing node (inputs and outputs)
    pub fn add_process_node(
        &mut self,
        name: impl Into<String>,
        input_types: Vec<(TypeId, String)>,
        output_types: Vec<(TypeId, String)>,
    ) -> NodeId {
        let id = NodeId::new(self.next_node_id);
        self.next_node_id += 1;

        let input_ports = input_types
            .into_iter()
            .map(|(type_id, type_name)| PortInfo { type_id, type_name })
            .collect();

        let output_ports = output_types
            .into_iter()
            .map(|(type_id, type_name)| PortInfo { type_id, type_name })
            .collect();

        self.nodes.insert(
            id,
            NodeInfo {
                name: name.into(),
                input_ports,
                output_ports,
            },
        );

        id
    }

    /// Connect two nodes with a typed channel
    ///
    /// # Parameters
    /// - `from_node`: Input consumed by this operation.
    /// - `from_port`: Input consumed by this operation.
    /// - `to_node`: Input consumed by this operation.
    /// - `to_port`: Input consumed by this operation.
    /// - `buffer_size`: Input consumed by this operation.
    pub fn connect<T: Send + 'static>(
        &mut self,
        from_node: NodeId,
        from_port: usize,
        to_node: NodeId,
        to_port: usize,
        buffer_size: usize,
    ) -> Result<(), Box<ConnectionError>> {
        // Validate nodes exist
        let from_info = self.nodes.get(&from_node).ok_or_else(|| {
            Box::new(ConnectionError::NodeNotFound(
                from_node.as_usize().to_string(),
            ))
        })?;
        let to_info = self.nodes.get(&to_node).ok_or_else(|| {
            Box::new(ConnectionError::NodeNotFound(
                to_node.as_usize().to_string(),
            ))
        })?;

        // Validate ports exist
        let from_port_info = from_info.output_ports.get(from_port).ok_or_else(|| {
            Box::new(ConnectionError::PortNotFound {
                node: from_info.name.clone(),
                port: from_port.to_string(),
            })
        })?;
        let to_port_info = to_info.input_ports.get(to_port).ok_or_else(|| {
            Box::new(ConnectionError::PortNotFound {
                node: to_info.name.clone(),
                port: to_port.to_string(),
            })
        })?;

        // Validate types match
        let expected_type = TypeId::of::<T>();
        if from_port_info.type_id != expected_type {
            return Err(Box::new(ConnectionError::PortTypeMismatch {
                node: from_info.name.clone(),
                port: from_port.to_string(),
                requested: std::any::type_name::<T>().to_string(),
                actual: from_port_info.type_name.clone(),
            }));
        }
        if to_port_info.type_id != expected_type {
            return Err(Box::new(ConnectionError::PortTypeMismatch {
                node: to_info.name.clone(),
                port: to_port.to_string(),
                requested: std::any::type_name::<T>().to_string(),
                actual: to_port_info.type_name.clone(),
            }));
        }

        self.connections.push(Connection {
            from_node,
            from_port,
            to_node,
            to_port,
            buffer_size,
            type_id: expected_type,
        });

        Ok(())
    }

    /// Get information about a node
    pub fn node_info(&self, node_id: NodeId) -> Option<(&str, usize, usize)> {
        self.nodes.get(&node_id).map(|info| {
            (
                info.name.as_str(),
                info.input_ports.len(),
                info.output_ports.len(),
            )
        })
    }

    /// Get all connections in the graph
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Get the number of nodes
    pub fn num_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// Validate the graph (check all ports are connected, no cycles for now)
    pub fn validate(&self) -> Result<(), Box<ConnectionError>> {
        // For now, just ensure inputs and outputs are properly connected
        // More sophisticated validation (cycle checking, etc.) can be added later

        for (node_id, node_info) in &self.nodes {
            // Check that all input ports are connected
            for input_port in 0..node_info.input_ports.len() {
                let connected = self
                    .connections
                    .iter()
                    .any(|conn| conn.to_node == *node_id && conn.to_port == input_port);

                if !connected {
                    return Err(Box::new(ConnectionError::UnconnectedInput {
                        node: node_info.name.clone(),
                        port: input_port.to_string(),
                    }));
                }
            }

            // Note: Output ports don't need to be connected (e.g., for debugging taps)
        }

        Ok(())
    }
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_graph_building() {
        let mut builder = GraphBuilder::new();

        let source = builder.add_process_node(
            "source",
            vec![],
            vec![(TypeId::of::<u32>(), "u32".to_string())],
        );
        let sink = builder.add_process_node(
            "sink",
            vec![(TypeId::of::<u32>(), "u32".to_string())],
            vec![],
        );

        assert!(builder.connect::<u32>(source, 0, sink, 0, 1000).is_ok());
        assert!(builder.validate().is_ok());
    }

    #[test]
    fn test_type_mismatch() {
        let mut builder = GraphBuilder::new();

        let source = builder.add_process_node(
            "source",
            vec![],
            vec![(TypeId::of::<u32>(), "u32".to_string())],
        );
        let sink = builder.add_process_node(
            "sink",
            vec![(TypeId::of::<u64>(), "u64".to_string())],
            vec![],
        );

        assert_eq!(
            *builder
                .connect::<u32>(source, 0, sink, 0, 1000)
                .unwrap_err(),
            ConnectionError::PortTypeMismatch {
                node: "sink".to_string(),
                port: "0".to_string(),
                requested: "u32".to_string(),
                actual: "u64".to_string(),
            }
        );
    }

    #[test]
    fn test_invalid_port() {
        let mut builder = GraphBuilder::new();

        let source = builder.add_process_node(
            "source",
            vec![],
            vec![(TypeId::of::<u32>(), "u32".to_string())],
        );
        let sink = builder.add_process_node(
            "sink",
            vec![(TypeId::of::<u32>(), "u32".to_string())],
            vec![],
        );

        // Try to connect to non-existent port
        assert_eq!(
            *builder
                .connect::<u32>(source, 1, sink, 0, 1000)
                .unwrap_err(),
            ConnectionError::PortNotFound {
                node: "source".to_string(),
                port: "1".to_string(),
            }
        );
    }
}
