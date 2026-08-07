//! Generic split-panel layout for egui applications.
//!
//! Panel and content identifiers are opaque strings. The manager owns the
//! split tree, boundary menus, split placement, content selection, dragging,
//! closing, and maximizing. Hosts provide content descriptions and render
//! arbitrary title-bar and body widgets through [`PanelSlot`].
//!
//! The split tree and interaction mechanics are reusable; application panel
//! identity, content behavior, and command policy remain with the host.

mod contract;
mod controls;
mod geometry;
mod icon;
mod layout;
mod tree;

pub use contract::{BoundaryInteraction, PanelGeometry, PanelLayoutResponse, PanelSlot, PanelSpec};
pub use icon::PanelIcon;
pub use layout::{
    LayoutNode, PanelLayout, PanelLayoutState, PanelLayoutStyle, PanelState, SplitAxis,
    TitleBarPosition, VerticalPanelLayout,
};
