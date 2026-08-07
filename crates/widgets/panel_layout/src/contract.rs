//! Host-facing panel declarations and render results.
//!
//! These records form the application-neutral contract between a host and the
//! layout manager. They describe content and report geometry without exposing
//! tree mutation or interaction implementation details.

use egui::Rect;

use super::icon::PanelIcon;
use super::layout::TitleBarPosition;

#[derive(Debug, Clone, Copy)]
pub struct PanelSpec<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub icon: PanelIcon,
    pub minimum_width: f32,
    pub minimum_height: f32,
    pub singleton: bool,
}

impl<'a> PanelSpec<'a> {
    /// Creates a panel specification with a default icon and non-singleton behavior.
    ///
    /// # Parameters
    /// - `id`: Opaque host identifier for the panel type.
    /// - `title`: Title rendered in the panel's title bar.
    /// - `minimum_height`: Smallest usable height in logical points. It also initializes the
    ///   minimum width; use [`Self::minimum_width`] to choose a different width.
    pub const fn new(id: &'a str, title: &'a str, minimum_height: f32) -> Self {
        Self {
            id,
            title,
            icon: PanelIcon::Panel,
            minimum_width: minimum_height,
            minimum_height,
            singleton: false,
        }
    }

    /// Sets the panel's minimum width in logical points.
    ///
    /// # Parameters
    /// - `minimum_width`: Smallest usable width of this panel.
    pub const fn minimum_width(mut self, minimum_width: f32) -> Self {
        self.minimum_width = minimum_width;
        self
    }

    /// Selects the icon shown for this panel in content-selection menus.
    ///
    /// # Parameters
    /// - `icon`: Application-neutral icon selected by the host.
    pub const fn icon(mut self, icon: PanelIcon) -> Self {
        self.icon = icon;
        self
    }

    /// Marks the panel as allowing at most one visible instance.
    pub const fn singleton(mut self) -> Self {
        self.singleton = true;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSlot<'a> {
    TitleBar {
        panel_id: &'a str,
        content_id: &'a str,
    },
    Body {
        panel_id: &'a str,
        content_id: &'a str,
    },
}

#[derive(Debug, Clone)]
pub struct PanelGeometry {
    pub panel_id: String,
    pub content_id: String,
    pub title_rect: Rect,
    /// Empty title-bar area that accepts area-level mouse gestures. Content
    /// selectors, host-provided text/buttons, and panel controls are excluded.
    pub title_interaction_rect: Option<Rect>,
    pub body_rect: Rect,
    pub panel_rect: Rect,
    pub allocated_rect: Rect,
    pub title_bar_position: TitleBarPosition,
    pub maximized: bool,
}

#[derive(Debug, Clone)]
pub struct PanelLayoutResponse {
    pub panels: Vec<PanelGeometry>,
    pub footer_rect: Rect,
    pub boundary_interaction: Option<BoundaryInteraction>,
    pub boundary_break_available: bool,
}

impl PanelLayoutResponse {
    /// Finds geometry for the panel instance with this stable layout identifier.
    ///
    /// # Parameters
    /// - `panel_id`: Persisted panel-instance identifier.
    pub fn panel(&self, panel_id: &str) -> Option<&PanelGeometry> {
        self.panels.iter().find(|panel| panel.panel_id == panel_id)
    }

    /// Finds geometry for the first panel displaying a content identifier.
    ///
    /// # Parameters
    /// - `content_id`: Opaque host content identifier.
    pub fn content_panel(&self, content_id: &str) -> Option<&PanelGeometry> {
        self.panels
            .iter()
            .find(|panel| panel.content_id == content_id)
    }
}

/// Pointer interaction currently taking place on a boundary between panels.
///
/// Hosts can use this application-neutral state to select an input-binding
/// context for status hints without teaching the layout manager about those
/// bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryInteraction {
    Hovered,
    Dragging,
    DraggingWithParallelBoundary,
}
