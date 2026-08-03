use egui::Color32;
use serde::{Deserialize, Serialize};

use super::ids::NodeId;

/// Stable identity of a visual graph frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameId(pub u32);

/// User-arranged visual grouping of graph nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    /// Stable persisted frame identity.
    pub id: FrameId,
    /// User-editable frame label.
    pub label: String,
    /// Display color used for the frame.
    pub color: Color32,
    /// Nodes visually enclosed by the frame.
    pub node_ids: Vec<NodeId>,
    #[serde(default)]
    /// Whether the frame is selected in the editor.
    pub selected: bool,
}
