//! Split-panel state and interaction orchestration.
//!
//! This module owns [`PanelLayout`]'s private persisted and transient state and
//! orders host synchronization, interaction reduction, and the render pass.
//! Consumers use it through the crate-root `PanelLayout` facade. It consumes
//! host-facing records plus the sibling geometry, tree, icon, and control
//! owners. It does not own pure geometry, tree traversal and mutation, icon or
//! control rendering, or the application meaning of panel identifiers.

use std::collections::HashSet;

use egui::{
    Color32, CornerRadius, CursorIcon, KeyboardShortcut, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder,
};
use serde::{Deserialize, Serialize};

use input_bindings::MenuShortcut;
use widget_support::menu_item_layout_job;

use super::contract::{
    BoundaryInteraction, PanelGeometry, PanelLayoutResponse, PanelSlot, PanelSpec,
};
use super::controls::{PanelControlIcon, panel_content_button, panel_control_button};
use super::geometry::{
    BOUNDARY_EXTEND_DISTANCE, BoundaryGeometry, adjacent_panels_at_boundary, axis_coordinate,
    boundary_break_candidate, collect_geometries, find_content_split_fraction, fraction_in_rect,
    paint_extended_boundary_guide, panel_at_pointer, push_panel_geometry, title_interaction_rect,
};
use super::icon::PanelIcon;
use super::tree::{
    all_panels, assigned_singletons, available_content, break_split, build_panel_column,
    build_vertical_tree, find_panel, find_panel_by_content, find_panel_mut, join_split,
    max_layout_id, remove_panel, set_split_fraction, split_content_by_content, split_panel,
    subtree_contains_only, swap_panel_contents, visit_panels_mut,
};

type BoundaryHandling = (
    Vec<LayoutAction>,
    Option<BoundaryInteraction>,
    Option<(SplitAxis, f32)>,
    bool,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelState {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub title_bar_position: TitleBarPosition,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleBarPosition {
    #[default]
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    /// A horizontal boundary with panels above and below it.
    Horizontal,
    /// A vertical boundary with panels to its left and right.
    Vertical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayoutNode {
    Panel {
        panel: PanelState,
    },
    Split {
        id: u64,
        axis: SplitAxis,
        fraction: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PanelLayoutState {
    #[serde(default)]
    root: Option<LayoutNode>,
    #[serde(default)]
    maximized: Option<String>,
    #[serde(default = "default_next_id")]
    next_id: u64,
}

fn default_next_id() -> u64 {
    1
}

#[derive(Debug, Clone)]
pub struct PanelLayoutStyle {
    pub title_height: f32,
    pub splitter_size: f32,
    pub splitter_visual_size: f32,
    pub outer_margin: f32,
    pub corner_radius: u8,
    pub title_fill: Color32,
    pub title_hover_fill: Color32,
    pub panel_fill: Color32,
    pub border_color: Color32,
    pub splitter_fill: Color32,
    pub splitter_drag_fill: Color32,
}

impl Default for PanelLayoutStyle {
    fn default() -> Self {
        Self {
            title_height: 28.0,
            splitter_size: 4.0,
            splitter_visual_size: 2.0,
            outer_margin: 4.0,
            corner_radius: 7,
            title_fill: Color32::from_rgb(38, 38, 38),
            title_hover_fill: Color32::from_rgb(47, 47, 47),
            panel_fill: Color32::from_rgb(28, 28, 28),
            border_color: Color32::from_rgb(78, 78, 78),
            splitter_fill: Color32::from_rgb(16, 16, 16),
            splitter_drag_fill: Color32::from_rgb(90, 90, 90),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PanelLayout {
    state: PanelLayoutState,
    style: PanelLayoutStyle,
    boundary_context: Option<BoundaryContext>,
    split_placement: Option<SplitPlacement>,
    maximize_shortcut: Option<KeyboardShortcut>,
}

impl PanelLayout {
    /// Creates a top-to-bottom layout. The supplied weights determine the
    /// initial horizontal split fractions.
    ///
    /// # Parameters
    /// - `panels`: Initial `(content identifier, relative weight)` pairs in top-to-bottom order.
    ///   Weights are clamped to a small positive value before fractions are computed.
    pub fn new(panels: impl IntoIterator<Item = (impl Into<String>, f32)>) -> Self {
        let panels: Vec<_> = panels
            .into_iter()
            .map(|(content, weight)| (content.into(), weight.max(0.001)))
            .collect();
        let mut next_id = 1;
        let root = build_vertical_tree(&panels, &mut next_id);
        Self {
            state: PanelLayoutState {
                root,
                next_id,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Restores a layout from persisted state and repairs its next unique identifier.
    ///
    /// # Parameters
    /// - `state`: Previously serialized layout tree.
    pub fn from_state(mut state: PanelLayoutState) -> Self {
        state.next_id = state
            .next_id
            .max(max_layout_id(state.root.as_ref()).saturating_add(1))
            .max(1);
        Self {
            state,
            ..Default::default()
        }
    }

    /// Returns the persisted split tree and maximized-panel state.
    pub fn state(&self) -> &PanelLayoutState {
        &self.state
    }

    /// Replaces the visual metrics and colors used by subsequent rendering.
    ///
    /// # Parameters
    /// - `style`: Complete visual style to apply.
    pub fn set_style(&mut self, style: PanelLayoutStyle) {
        self.style = style;
    }

    /// Configures the keyboard shortcut that toggles the area under the
    /// pointer between maximized and restored layout states.
    ///
    /// # Parameters
    /// - `shortcut`: Shortcut to consume, or `None` to disable this interaction.
    pub fn set_maximize_shortcut(&mut self, shortcut: Option<KeyboardShortcut>) {
        self.maximize_shortcut = shortcut;
    }

    /// Returns the fraction of the split directly separating two content identifiers.
    ///
    /// # Parameters
    /// - `first_content`: Content expected on the first side of the split.
    /// - `second_content`: Content expected on the second side of the split.
    pub fn split_fraction(&self, first_content: &str, second_content: &str) -> Option<f32> {
        find_content_split_fraction(self.state.root.as_ref()?, first_content, second_content)
    }

    /// Ensures `content_id` is visible in a top-to-bottom column on the right
    /// side of the complete layout.
    ///
    /// If a right column containing only `ordered_contents` already exists,
    /// the new content is inserted there and the column is rebuilt in the
    /// supplied order. Otherwise the complete current layout is wrapped in a
    /// new vertical split. Identifiers and ordering are opaque to the manager.
    ///
    /// # Parameters
    /// - `content_id`: Content to add when it is not already present.
    /// - `ordered_contents`: Complete intended order for the auxiliary column.
    /// - `existing_layout_fraction`: Fraction retained by the existing layout when a new column
    ///   must be created.
    ///
    /// Returns whether the layout changed.
    pub fn ensure_right_column_content(
        &mut self,
        content_id: &str,
        ordered_contents: &[&str],
        existing_layout_fraction: f32,
    ) -> bool {
        if !ordered_contents.contains(&content_id)
            || find_panel_by_content(self.state.root.as_ref(), content_id).is_some()
        {
            return false;
        }
        self.add_right_column_content(content_id, ordered_contents, existing_layout_fraction)
    }

    /// Ensures `content_id` is present immediately beside `anchor_content`.
    ///
    /// When the anchor is visible, the new panel replaces its leaf with the
    /// requested split. This preserves the anchor's enclosing layout, such as
    /// a primary panel area beside an auxiliary column. If the anchor is no
    /// longer present, the new panel wraps the current layout at the requested
    /// edge instead.
    ///
    /// # Parameters
    /// - `content_id`: Content to add when it is not already present.
    /// - `anchor_content`: Existing content to split beside when available.
    /// - `axis`: Orientation of the new split.
    /// - `content_first`: Whether the new content occupies the split's first side.
    /// - `fraction`: Space assigned to the first side, clamped to `0.1..=0.9`.
    ///
    /// Returns whether the layout changed.
    pub fn ensure_adjacent_content(
        &mut self,
        content_id: &str,
        anchor_content: &str,
        axis: SplitAxis,
        content_first: bool,
        fraction: f32,
    ) -> bool {
        if find_panel_by_content(self.state.root.as_ref(), content_id).is_some() {
            return false;
        }

        self.restore_maximized();
        let new_panel = PanelState {
            id: self.allocate_id("panel"),
            content: content_id.to_owned(),
            title_bar_position: TitleBarPosition::Top,
        };
        let split_id = self.allocate_numeric_id();
        let fraction = fraction.clamp(0.1, 0.9);

        if split_content_by_content(
            self.state.root.as_mut(),
            anchor_content,
            axis,
            content_first,
            fraction,
            split_id,
            new_panel.clone(),
        ) {
            return true;
        }

        let Some(existing) = self.state.root.take() else {
            self.state.root = Some(LayoutNode::Panel { panel: new_panel });
            return true;
        };
        let new = LayoutNode::Panel { panel: new_panel };
        let (first, second) = if content_first {
            (new, existing)
        } else {
            (existing, new)
        };
        self.state.root = Some(LayoutNode::Split {
            id: split_id,
            axis,
            fraction,
            first: Box::new(first),
            second: Box::new(second),
        });
        true
    }

    /// Ensures that at least `count` panels with `content_id` are present.
    /// Additional instances are placed in the same ordered right-side column.
    ///
    /// # Parameters
    /// - `content_id`: Content whose visible instance count is enforced.
    /// - `count`: Minimum number of instances to create.
    /// - `ordered_contents`: Complete intended order for the auxiliary column.
    /// - `existing_layout_fraction`: Fraction retained by the existing layout when a new column
    ///   must be created.
    ///
    /// Returns whether one or more instances were added.
    pub fn ensure_right_column_content_count(
        &mut self,
        content_id: &str,
        count: usize,
        ordered_contents: &[&str],
        existing_layout_fraction: f32,
    ) -> bool {
        if !ordered_contents.contains(&content_id) {
            return false;
        }
        let existing = all_panels(self.state.root.as_ref())
            .into_iter()
            .filter(|panel| panel.content == content_id)
            .count();
        let mut changed = false;
        for _ in existing..count {
            changed |= self.add_right_column_content(
                content_id,
                ordered_contents,
                existing_layout_fraction,
            );
        }
        changed
    }

    fn add_right_column_content(
        &mut self,
        content_id: &str,
        ordered_contents: &[&str],
        existing_layout_fraction: f32,
    ) -> bool {
        self.restore_maximized();

        let existing_column = match self.state.root.as_ref() {
            Some(LayoutNode::Split {
                axis: SplitAxis::Vertical,
                second,
                ..
            }) if subtree_contains_only(second, ordered_contents) => Some(
                all_panels(Some(second))
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let new_panel = PanelState {
            id: self.allocate_id("panel"),
            content: content_id.to_owned(),
            title_bar_position: TitleBarPosition::Top,
        };

        if let Some(mut panels) = existing_column {
            panels.push(new_panel);
            panels.sort_by_key(|panel| {
                ordered_contents
                    .iter()
                    .position(|content| *content == panel.content)
                    .unwrap_or(usize::MAX)
            });
            let split_ids: Vec<_> = (1..panels.len())
                .map(|_| self.allocate_numeric_id())
                .collect();
            let column = build_panel_column(&panels, &split_ids);
            let Some(LayoutNode::Split { second, .. }) = self.state.root.as_mut() else {
                return false;
            };
            **second = column;
        } else if let Some(existing) = self.state.root.take() {
            let split_id = self.allocate_numeric_id();
            self.state.root = Some(LayoutNode::Split {
                id: split_id,
                axis: SplitAxis::Vertical,
                fraction: existing_layout_fraction.clamp(0.1, 0.9),
                first: Box::new(existing),
                second: Box::new(LayoutNode::Panel { panel: new_panel }),
            });
        } else {
            self.state.root = Some(LayoutNode::Panel { panel: new_panel });
        }
        true
    }

    /// Synchronizes declared content and renders the complete split-panel layout.
    ///
    /// # Parameters
    /// - `ui`: Parent UI used for painting and pointer input.
    /// - `rect`: Allocated layout rectangle in screen coordinates.
    /// - `footer_height`: Height reserved below panels for host-owned footer content.
    /// - `specs`: Currently available panel content declarations.
    /// - `add_widget`: Host callback used to render each title-bar and body slot.
    ///
    /// Returns panel geometry and boundary-interaction information for host input handling.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        footer_height: f32,
        specs: &[PanelSpec<'_>],
        mut add_widget: impl FnMut(PanelSlot<'_>, &mut Ui),
    ) -> PanelLayoutResponse {
        self.synchronize(specs);
        let layout_rect = Rect::from_min_max(
            rect.min,
            egui::pos2(
                rect.right(),
                (rect.bottom() - footer_height).max(rect.top()),
            ),
        );
        let footer_rect = Rect::from_min_max(layout_rect.left_bottom(), rect.right_bottom());
        let root_rect = layout_rect.shrink2(egui::vec2(self.style.outer_margin, 0.0));

        let (mut base_geometries, mut boundaries) = self.geometries(root_rect, specs);
        let mut geometries = base_geometries.clone();
        let mut actions = Vec::new();
        let mut boundary_interaction = None;
        let mut extended_boundary_guide = None;
        let mut boundary_break_available = false;
        if self.split_placement.is_some() {
            if let Some(preview_action) =
                self.split_action_at_pointer(ui, &base_geometries, root_rect)
            {
                (geometries, boundaries) =
                    self.split_preview_geometries(root_rect, specs, preview_action);
            }
            self.paint_boundaries(ui, &boundaries);
        } else {
            (
                actions,
                boundary_interaction,
                extended_boundary_guide,
                boundary_break_available,
            ) = self.handle_boundaries(ui, &boundaries, &base_geometries, root_rect);
            if actions
                .iter()
                .any(|action| matches!(action, LayoutAction::SetFraction { .. }))
            {
                for action in actions.drain(..) {
                    self.apply_action(action, specs);
                }
                (base_geometries, boundaries) = self.geometries(root_rect, specs);
                geometries.clone_from(&base_geometries);
            }
        }
        if self.split_placement.is_none()
            && let Some(action) = self.maximize_shortcut_action(ui, &base_geometries)
        {
            actions.push(action);
        }

        for geometry in &mut geometries {
            let (action, interaction_rect) =
                self.show_title_bar(ui, specs, geometry, &mut add_widget);
            geometry.title_interaction_rect = interaction_rect;
            if let Some(action) = action {
                actions.push(action);
            }
            let mut body_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt(("panel-body", geometry.panel_id.as_str()))
                    .max_rect(geometry.body_rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            body_ui.set_clip_rect(geometry.body_rect);
            add_widget(
                PanelSlot::Body {
                    panel_id: &geometry.panel_id,
                    content_id: &geometry.content_id,
                },
                &mut body_ui,
            );
            self.finish_panel(ui, geometry);
        }
        if let Some(action) = self.handle_split_placement(ui, &base_geometries, root_rect) {
            actions.push(action);
        }

        for action in actions {
            self.apply_action(action, specs);
        }
        if let Some((axis, coordinate)) = extended_boundary_guide {
            paint_extended_boundary_guide(ui, axis, root_rect, coordinate);
        }

        // Keep boundary geometry alive through the complete pass. egui's
        // context-menu state is tied to the stable split IDs, not this vector.
        drop(boundaries);
        PanelLayoutResponse {
            panels: geometries,
            footer_rect,
            boundary_interaction,
            boundary_break_available,
        }
    }

    fn synchronize(&mut self, specs: &[PanelSpec<'_>]) {
        if specs.is_empty() {
            self.state.root = None;
            self.state.maximized = None;
            return;
        }
        if self.state.root.is_none() {
            self.state.root = Some(LayoutNode::Panel {
                panel: PanelState {
                    id: specs[0].id.to_owned(),
                    content: specs[0].id.to_owned(),
                    title_bar_position: TitleBarPosition::Top,
                },
            });
        }

        let valid: HashSet<_> = specs.iter().map(|spec| spec.id).collect();
        let mut assigned_singletons = HashSet::new();
        visit_panels_mut(self.state.root.as_mut(), &mut |panel| {
            let current = specs.iter().find(|spec| spec.id == panel.content);
            let duplicate_singleton = current.is_some_and(|spec| {
                spec.singleton && !assigned_singletons.insert(spec.id.to_owned())
            });
            if !valid.contains(panel.content.as_str()) || duplicate_singleton {
                panel.content = available_content(specs, &assigned_singletons)
                    .unwrap_or(specs[0].id)
                    .to_owned();
                if specs
                    .iter()
                    .any(|spec| spec.id == panel.content && spec.singleton)
                {
                    assigned_singletons.insert(panel.content.clone());
                }
            }
        });

        if self
            .state
            .maximized
            .as_ref()
            .is_some_and(|id| find_panel(self.state.root.as_ref(), id).is_none())
        {
            self.state.maximized = None;
        }
    }

    fn geometries(
        &self,
        rect: Rect,
        specs: &[PanelSpec<'_>],
    ) -> (Vec<PanelGeometry>, Vec<BoundaryGeometry>) {
        let mut panels = Vec::new();
        let mut boundaries = Vec::new();
        let Some(root) = self.state.root.as_ref() else {
            return (panels, boundaries);
        };
        if let Some(maximized) = self.state.maximized.as_deref()
            && let Some(panel) = find_panel(Some(root), maximized)
        {
            push_panel_geometry(panel, rect, true, &self.style, &mut panels);
            return (panels, boundaries);
        }
        collect_geometries(root, rect, specs, &self.style, &mut panels, &mut boundaries);
        (panels, boundaries)
    }

    fn handle_boundaries(
        &mut self,
        ui: &mut Ui,
        boundaries: &[BoundaryGeometry],
        panels: &[PanelGeometry],
        root_rect: Rect,
    ) -> BoundaryHandling {
        let mut actions = Vec::new();
        let mut interaction = None;
        let mut extended_boundary_guide = None;
        let mut break_available = false;
        for boundary in boundaries {
            let break_candidate =
                boundary_break_candidate(self.state.root.as_ref(), boundary.id, boundaries);
            let response = ui.interact(
                boundary.rect,
                ui.id().with(("panel-splitter", boundary.id)),
                Sense::click_and_drag(),
            );
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(match boundary.axis {
                    SplitAxis::Horizontal => CursorIcon::ResizeVertical,
                    SplitAxis::Vertical => CursorIcon::ResizeHorizontal,
                });
            }
            if response.dragged() {
                break_available = break_candidate.is_some();
                interaction = Some(
                    if boundary
                        .nearest_parallel_boundary(boundaries, BOUNDARY_EXTEND_DISTANCE)
                        .is_some()
                    {
                        BoundaryInteraction::DraggingWithParallelBoundary
                    } else {
                        BoundaryInteraction::Dragging
                    },
                );
            } else if response.hovered() && interaction.is_none() {
                interaction = Some(BoundaryInteraction::Hovered);
            }
            if response.dragged()
                && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            {
                let modifiers = ui.input(|input| input.modifiers);
                if modifiers.alt
                    && let Some(candidate) = break_candidate
                {
                    actions.push(LayoutAction::BreakSplit {
                        split_id: boundary.id,
                        band: if axis_coordinate(candidate.axis, pointer) < candidate.coordinate {
                            SplitSide::First
                        } else {
                            SplitSide::Second
                        },
                        crossing_fraction: candidate.crossing_fraction,
                    });
                }
                let resize_actions = boundary.resize_actions(
                    pointer,
                    boundaries,
                    root_rect,
                    modifiers.ctrl,
                    modifiers.shift && !modifiers.alt,
                );
                if modifiers.shift
                    && !modifiers.alt
                    && resize_actions.len() > 1
                    && let Some(LayoutAction::SetFraction { fraction, .. }) = resize_actions.first()
                {
                    extended_boundary_guide =
                        Some((boundary.axis, boundary.coordinate_for_fraction(*fraction)));
                }
                actions.extend(resize_actions);
            }
            if response.secondary_clicked() {
                let pointer = ui.input(|input| input.pointer.interact_pos());
                self.boundary_context = Some(BoundaryContext {
                    split_id: boundary.id,
                    axis: boundary.axis,
                    adjacent_panels: pointer
                        .and_then(|pointer| adjacent_panels_at_boundary(panels, boundary, pointer)),
                });
            }

            response.context_menu(|ui| {
                let Some(context) = self
                    .boundary_context
                    .as_ref()
                    .filter(|context| context.split_id == boundary.id)
                    .cloned()
                else {
                    ui.close();
                    return;
                };
                let ((first_label, first_keep), (second_label, second_keep)) =
                    join_options(context.axis);
                if ui.button(first_label).clicked() {
                    actions.push(LayoutAction::Join {
                        split_id: context.split_id,
                        keep: first_keep,
                    });
                    ui.close();
                }
                if ui.button(second_label).clicked() {
                    actions.push(LayoutAction::Join {
                        split_id: context.split_id,
                        keep: second_keep,
                    });
                    ui.close();
                }
                if ui
                    .add_enabled(
                        context.adjacent_panels.is_some(),
                        egui::Button::new("Swap Content"),
                    )
                    .clicked()
                {
                    let Some((first_panel_id, second_panel_id)) = context.adjacent_panels else {
                        return;
                    };
                    actions.push(LayoutAction::SwapContent {
                        first_panel_id,
                        second_panel_id,
                    });
                    ui.close();
                }
                ui.separator();
                if ui.button("Horizontal Split").clicked() {
                    self.split_placement = Some(SplitPlacement::Panel {
                        axis: SplitAxis::Horizontal,
                    });
                    ui.close();
                }
                if ui.button("Vertical Split").clicked() {
                    self.split_placement = Some(SplitPlacement::Panel {
                        axis: SplitAxis::Vertical,
                    });
                    ui.close();
                }
            });

            ui.painter()
                .rect_filled(boundary.rect, 0.0, self.style.splitter_fill);
            if response.dragged() {
                let visual = boundary.visual_rect(self.style.splitter_visual_size);
                ui.painter()
                    .rect_filled(visual, 0.0, self.style.splitter_drag_fill);
            }
        }
        (
            actions,
            interaction,
            extended_boundary_guide,
            break_available,
        )
    }

    fn handle_split_placement(
        &mut self,
        ui: &mut Ui,
        panels: &[PanelGeometry],
        root_rect: Rect,
    ) -> Option<LayoutAction> {
        let placement = self.split_placement?;
        if ui.input(|input| {
            input.key_pressed(egui::Key::Escape) || input.pointer.secondary_clicked()
        }) {
            self.split_placement = None;
            return None;
        }

        match placement {
            SplitPlacement::Panel { axis } => {
                for panel in panels {
                    let response = ui.interact(
                        panel.panel_rect,
                        ui.id()
                            .with(("panel-split-placement", panel.panel_id.as_str())),
                        Sense::click(),
                    );
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(split_cursor(axis));
                    }
                    if response.clicked()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        self.split_placement = None;
                        return Some(LayoutAction::Split {
                            panel_id: panel.panel_id.clone(),
                            axis,
                            fraction: fraction_in_rect(axis, panel.panel_rect, pointer),
                        });
                    }
                }
            }
            SplitPlacement::Layout { side } => {
                let response = ui.interact(
                    root_rect,
                    ui.id().with("layout-split-placement"),
                    Sense::click(),
                );
                if response.hovered() {
                    ui.ctx().set_cursor_icon(split_cursor(side.axis()));
                }
                if response.clicked()
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    self.split_placement = None;
                    return Some(LayoutAction::SplitLayout {
                        side,
                        fraction: fraction_in_rect(side.axis(), root_rect, pointer),
                    });
                }
            }
        }
        None
    }

    fn split_action_at_pointer(
        &self,
        ui: &Ui,
        panels: &[PanelGeometry],
        root_rect: Rect,
    ) -> Option<LayoutAction> {
        let placement = self.split_placement.as_ref()?;
        let pointer = ui.input(|input| input.pointer.hover_pos())?;
        match *placement {
            SplitPlacement::Panel { axis } => {
                let panel = panel_at_pointer(panels, pointer)?;
                Some(LayoutAction::Split {
                    panel_id: panel.panel_id.clone(),
                    axis,
                    fraction: fraction_in_rect(axis, panel.panel_rect, pointer),
                })
            }
            SplitPlacement::Layout { side } if root_rect.contains(pointer) => {
                Some(LayoutAction::SplitLayout {
                    side,
                    fraction: fraction_in_rect(side.axis(), root_rect, pointer),
                })
            }
            SplitPlacement::Layout { .. } => None,
        }
    }

    fn split_preview_geometries(
        &self,
        rect: Rect,
        specs: &[PanelSpec<'_>],
        action: LayoutAction,
    ) -> (Vec<PanelGeometry>, Vec<BoundaryGeometry>) {
        let mut preview = self.clone();
        preview.split_placement = None;
        preview.apply_action(action, specs);
        preview.geometries(rect, specs)
    }

    fn paint_boundaries(&self, ui: &Ui, boundaries: &[BoundaryGeometry]) {
        for boundary in boundaries {
            ui.painter()
                .rect_filled(boundary.rect, 0.0, self.style.splitter_fill);
        }
    }

    fn maximize_shortcut_action(
        &self,
        ui: &mut Ui,
        panels: &[PanelGeometry],
    ) -> Option<LayoutAction> {
        let shortcut = self.maximize_shortcut?;
        let panel_id = self.state.maximized.clone().or_else(|| {
            let pointer = ui.input(|input| input.pointer.hover_pos())?;
            Some(panel_at_pointer(panels, pointer)?.panel_id.clone())
        })?;
        if !ui.input_mut(|input| input.consume_shortcut(&shortcut)) {
            return None;
        }
        Some(LayoutAction::Panel {
            panel_id,
            action: if self.state.maximized.is_some() {
                PanelAction::RestoreMaximized
            } else {
                PanelAction::Maximize
            },
        })
    }

    fn show_title_bar(
        &self,
        ui: &mut Ui,
        specs: &[PanelSpec<'_>],
        geometry: &PanelGeometry,
        add_widget: &mut impl FnMut(PanelSlot<'_>, &mut Ui),
    ) -> (Option<LayoutAction>, Option<Rect>) {
        let rounding = match geometry.title_bar_position {
            TitleBarPosition::Top => CornerRadius {
                nw: self.style.corner_radius,
                ne: self.style.corner_radius,
                sw: 0,
                se: 0,
            },
            TitleBarPosition::Bottom => CornerRadius {
                nw: 0,
                ne: 0,
                sw: self.style.corner_radius,
                se: self.style.corner_radius,
            },
        };
        ui.painter()
            .rect_filled(geometry.title_rect, rounding, self.style.title_fill);
        let divider = match geometry.title_bar_position {
            TitleBarPosition::Top => [
                geometry.title_rect.left_bottom(),
                geometry.title_rect.right_bottom(),
            ],
            TitleBarPosition::Bottom => [
                geometry.title_rect.left_top(),
                geometry.title_rect.right_top(),
            ],
        };
        ui.painter()
            .line_segment(divider, Stroke::new(1.0, self.style.border_color));

        let mut action = None;
        let mut title_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(("panel-title-content", geometry.panel_id.as_str()))
                .max_rect(geometry.title_rect.shrink2(egui::vec2(6.0, 2.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let selected_spec = specs
            .iter()
            .find(|spec| spec.id == geometry.content_id)
            .copied();
        let selected_title = selected_spec.map_or(geometry.content_id.as_str(), |spec| spec.title);
        let selected_icon = selected_spec.map_or(PanelIcon::Panel, |spec| spec.icon);
        let selector =
            egui::ComboBox::from_id_salt(("panel-content-selector", geometry.panel_id.as_str()))
                .selected_text("   ")
                .width(44.0)
                .show_ui(&mut title_ui, |ui| {
                    ui.set_min_width(190.0);
                    for spec in specs {
                        let assigned_elsewhere = spec.singleton
                            && find_panel_by_content(self.state.root.as_ref(), spec.id)
                                .is_some_and(|panel| panel.id != geometry.panel_id);
                        let selected = spec.id == geometry.content_id;
                        if ui
                            .add_enabled(!assigned_elsewhere, panel_content_button(*spec, selected))
                            .clicked()
                        {
                            action = Some(LayoutAction::ChangeContent {
                                panel_id: geometry.panel_id.clone(),
                                content_id: spec.id.to_owned(),
                            });
                            ui.close();
                        }
                    }
                });
        let icon_color = title_ui
            .visuals()
            .widgets
            .style(&selector.response)
            .fg_stroke
            .color;
        let icon_rect = Rect::from_center_size(
            egui::pos2(
                selector.response.rect.left() + 12.0,
                selector.response.rect.center().y,
            ),
            egui::vec2(16.0, 16.0),
        );
        selected_icon.paint(&title_ui, icon_rect, icon_color);
        selector.response.on_hover_text(selected_title);
        add_widget(
            PanelSlot::TitleBar {
                panel_id: &geometry.panel_id,
                content_id: &geometry.content_id,
            },
            &mut title_ui,
        );
        let interaction_left = title_ui.available_rect_before_wrap().left();
        let maximize_response = title_ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let maximize_icon = if geometry.maximized {
                    PanelControlIcon::RestoreLayout
                } else {
                    PanelControlIcon::Maximize
                };
                let maximize_tooltip = if geometry.maximized {
                    "Restore panel layout"
                } else {
                    "Maximize panel"
                };
                let response = panel_control_button(ui, maximize_icon, maximize_tooltip);
                if response.clicked() {
                    action = Some(LayoutAction::Panel {
                        panel_id: geometry.panel_id.clone(),
                        action: if geometry.maximized {
                            PanelAction::RestoreMaximized
                        } else {
                            PanelAction::Maximize
                        },
                    });
                }
                response
            })
            .inner;
        let interaction_rect = title_interaction_rect(
            geometry.title_rect,
            interaction_left,
            maximize_response.rect.left(),
        );
        if let Some(interaction_rect) = interaction_rect {
            let response = ui.interact(
                interaction_rect,
                ui.id().with(("panel-title", geometry.panel_id.as_str())),
                Sense::click(),
            );
            if response.hovered() {
                ui.painter()
                    .rect_filled(interaction_rect, 0.0, self.style.title_hover_fill);
            }
            if response.double_clicked() {
                action = Some(LayoutAction::Panel {
                    panel_id: geometry.panel_id.clone(),
                    action: if geometry.maximized {
                        PanelAction::RestoreMaximized
                    } else {
                        PanelAction::Maximize
                    },
                });
            }
            response.context_menu(|ui| {
                self.show_area_menu(ui, geometry, &mut action);
            });
        }
        ui.painter()
            .line_segment(divider, Stroke::new(1.0, self.style.border_color));
        (action, interaction_rect)
    }

    fn show_area_menu(
        &self,
        ui: &mut Ui,
        geometry: &PanelGeometry,
        action: &mut Option<LayoutAction>,
    ) {
        ui.set_min_width(180.0);
        let flip_label = match geometry.title_bar_position {
            TitleBarPosition::Top => "Flip to Bottom",
            TitleBarPosition::Bottom => "Flip to Top",
        };
        let flip_icon = match geometry.title_bar_position {
            TitleBarPosition::Top => "↓",
            TitleBarPosition::Bottom => "↑",
        };
        if area_menu_button(ui, flip_label, flip_icon, None, true).clicked() {
            *action = Some(LayoutAction::Panel {
                panel_id: geometry.panel_id.clone(),
                action: PanelAction::FlipTitleBar,
            });
            ui.close();
        }
        ui.separator();
        let split_label = menu_item_layout_job(ui, Some("▣"), "Split This Area");
        ui.menu_button(egui::WidgetText::LayoutJob(split_label.into()), |ui| {
            ui.set_min_width(180.0);
            if area_menu_button(ui, "Horizontal Split", "═", None, true).clicked() {
                *action = Some(LayoutAction::BeginSplit {
                    axis: SplitAxis::Horizontal,
                });
                ui.close();
            }
            if area_menu_button(ui, "Vertical Split", "║", None, true).clicked() {
                *action = Some(LayoutAction::BeginSplit {
                    axis: SplitAxis::Vertical,
                });
                ui.close();
            }
        });
        let add_label = menu_item_layout_job(ui, Some("+"), "Add Area to Layout");
        ui.menu_button(egui::WidgetText::LayoutJob(add_label.into()), |ui| {
            ui.set_min_width(180.0);
            for (label, icon, side) in [
                ("Left", "←", LayoutSide::Left),
                ("Right", "→", LayoutSide::Right),
                ("Top", "↑", LayoutSide::Top),
                ("Bottom", "↓", LayoutSide::Bottom),
            ] {
                if area_menu_button(ui, label, icon, None, true).clicked() {
                    *action = Some(LayoutAction::BeginLayoutSplit { side });
                    ui.close();
                }
            }
        });
        let maximize_label = if geometry.maximized {
            "Restore Area"
        } else {
            "Maximize Area"
        };
        let maximize_icon = if geometry.maximized { "▣" } else { "□" };
        if area_menu_button(
            ui,
            maximize_label,
            maximize_icon,
            self.maximize_shortcut.map(MenuShortcut::from_keyboard),
            true,
        )
        .clicked()
        {
            *action = Some(LayoutAction::Panel {
                panel_id: geometry.panel_id.clone(),
                action: if geometry.maximized {
                    PanelAction::RestoreMaximized
                } else {
                    PanelAction::Maximize
                },
            });
            ui.close();
        }
        ui.separator();
        if area_menu_button(
            ui,
            "Close Area",
            "×",
            None,
            all_panels(self.state.root.as_ref()).len() > 1,
        )
        .on_disabled_hover_text("The last area cannot be closed")
        .clicked()
        {
            *action = Some(LayoutAction::Panel {
                panel_id: geometry.panel_id.clone(),
                action: PanelAction::Close,
            });
            ui.close();
        }
    }

    fn finish_panel(&self, ui: &Ui, geometry: &PanelGeometry) {
        let rounding = CornerRadius::same(self.style.corner_radius);
        if geometry.panel_rect.height() > f32::from(self.style.corner_radius) {
            let radius = f32::from(self.style.corner_radius);
            let (cap, cap_rounding) = match geometry.title_bar_position {
                TitleBarPosition::Top => (
                    Rect::from_min_max(
                        egui::pos2(
                            geometry.panel_rect.left(),
                            geometry.panel_rect.bottom() - radius,
                        ),
                        geometry.panel_rect.right_bottom(),
                    ),
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: self.style.corner_radius,
                        se: self.style.corner_radius,
                    },
                ),
                TitleBarPosition::Bottom => (
                    Rect::from_min_max(
                        geometry.panel_rect.left_top(),
                        egui::pos2(
                            geometry.panel_rect.right(),
                            geometry.panel_rect.top() + radius,
                        ),
                    ),
                    CornerRadius {
                        nw: self.style.corner_radius,
                        ne: self.style.corner_radius,
                        sw: 0,
                        se: 0,
                    },
                ),
            };
            ui.painter()
                .rect_filled(cap, cap_rounding, self.style.panel_fill);
        }
        ui.painter().rect_stroke(
            geometry.panel_rect,
            rounding,
            Stroke::new(1.0, self.style.border_color),
            StrokeKind::Inside,
        );
    }

    fn apply_action(&mut self, action: LayoutAction, specs: &[PanelSpec<'_>]) {
        match action {
            LayoutAction::SetFraction { split_id, fraction } => {
                set_split_fraction(self.state.root.as_mut(), split_id, fraction);
            }
            LayoutAction::Join { split_id, keep } => {
                if self.state.maximized.is_some() {
                    self.restore_maximized();
                }
                join_split(self.state.root.as_mut(), split_id, keep);
            }
            LayoutAction::SwapContent {
                first_panel_id,
                second_panel_id,
            } => {
                if self.state.maximized.is_some() {
                    self.restore_maximized();
                }
                swap_panel_contents(self.state.root.as_mut(), &first_panel_id, &second_panel_id);
            }
            LayoutAction::BreakSplit {
                split_id,
                band,
                crossing_fraction,
            } => {
                break_split(self.state.root.as_mut(), split_id, band, crossing_fraction);
            }
            LayoutAction::Split {
                panel_id,
                axis,
                fraction,
            } => {
                if self.state.maximized.is_some() {
                    self.restore_maximized();
                }
                let assigned = assigned_singletons(self.state.root.as_ref(), specs);
                let Some(content) = available_content(specs, &assigned).map(str::to_owned) else {
                    return;
                };
                let new_panel_id = self.allocate_id("panel");
                let split_id = self.allocate_numeric_id();
                split_panel(
                    self.state.root.as_mut(),
                    &panel_id,
                    axis,
                    fraction,
                    split_id,
                    PanelState {
                        id: new_panel_id,
                        content,
                        title_bar_position: TitleBarPosition::Top,
                    },
                );
            }
            LayoutAction::SplitLayout { side, fraction } => {
                if self.state.maximized.is_some() {
                    self.restore_maximized();
                }
                let assigned = assigned_singletons(self.state.root.as_ref(), specs);
                let Some(content) = available_content(specs, &assigned).map(str::to_owned) else {
                    return;
                };
                let new_panel_id = self.allocate_id("panel");
                let split_id = self.allocate_numeric_id();
                let Some(existing) = self.state.root.take() else {
                    return;
                };
                let new_panel = LayoutNode::Panel {
                    panel: PanelState {
                        id: new_panel_id,
                        content,
                        title_bar_position: TitleBarPosition::Top,
                    },
                };
                let (first, second) = if side.new_area_is_first() {
                    (new_panel, existing)
                } else {
                    (existing, new_panel)
                };
                self.state.root = Some(LayoutNode::Split {
                    id: split_id,
                    axis: side.axis(),
                    fraction: fraction.clamp(0.1, 0.9),
                    first: Box::new(first),
                    second: Box::new(second),
                });
            }
            LayoutAction::BeginSplit { axis } => {
                self.split_placement = Some(SplitPlacement::Panel { axis });
            }
            LayoutAction::BeginLayoutSplit { side } => {
                if self.state.maximized.is_some() {
                    self.restore_maximized();
                }
                self.split_placement = Some(SplitPlacement::Layout { side });
            }
            LayoutAction::ChangeContent {
                panel_id,
                content_id,
            } => {
                if let Some(panel) = find_panel_mut(self.state.root.as_mut(), &panel_id) {
                    panel.content = content_id;
                }
            }
            LayoutAction::Panel { panel_id, action } => {
                self.apply_panel_action(&panel_id, action);
            }
        }
    }

    fn apply_panel_action(&mut self, panel_id: &str, action: PanelAction) {
        match action {
            PanelAction::FlipTitleBar => {
                if let Some(panel) = find_panel_mut(self.state.root.as_mut(), panel_id) {
                    panel.title_bar_position = match panel.title_bar_position {
                        TitleBarPosition::Top => TitleBarPosition::Bottom,
                        TitleBarPosition::Bottom => TitleBarPosition::Top,
                    };
                }
            }
            PanelAction::Maximize => {
                self.state.maximized = Some(panel_id.to_owned());
            }
            PanelAction::RestoreMaximized => self.restore_maximized(),
            PanelAction::Close => {
                if all_panels(self.state.root.as_ref()).len() <= 1 {
                    return;
                }
                self.restore_maximized();
                remove_panel(self.state.root.as_mut(), panel_id);
            }
        }
    }

    fn restore_maximized(&mut self) {
        self.state.maximized = None;
    }

    fn allocate_numeric_id(&mut self) -> u64 {
        let id = self.state.next_id;
        self.state.next_id += 1;
        id
    }

    fn allocate_id(&mut self, prefix: &str) -> String {
        loop {
            let id = format!("{prefix}-{}", self.allocate_numeric_id());
            if find_panel(self.state.root.as_ref(), &id).is_none() {
                return id;
            }
        }
    }
}

fn area_menu_button(
    ui: &mut Ui,
    label: &str,
    icon: &str,
    shortcut: Option<MenuShortcut>,
    enabled: bool,
) -> egui::Response {
    let job = menu_item_layout_job(ui, Some(icon), label);
    let mut button = egui::Button::new(egui::WidgetText::LayoutJob(job.into()))
        .wrap_mode(egui::TextWrapMode::Extend);
    if let Some(shortcut) = shortcut {
        button = button.right_text(shortcut.format(ui.ctx().os().is_mac()));
    }
    ui.add_enabled(enabled, button)
}

/// Compatibility alias for hosts that used the original flat vertical
/// manager. The implementation now supports arbitrary nested splits.
pub type VerticalPanelLayout = PanelLayout;

#[derive(Debug, Clone)]
struct BoundaryContext {
    split_id: u64,
    axis: SplitAxis,
    adjacent_panels: Option<(String, String)>,
}

#[derive(Debug, Clone, Copy)]
enum SplitPlacement {
    Panel { axis: SplitAxis },
    Layout { side: LayoutSide },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl LayoutSide {
    const fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Vertical,
            Self::Top | Self::Bottom => SplitAxis::Horizontal,
        }
    }

    const fn new_area_is_first(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LayoutAction {
    SetFraction {
        split_id: u64,
        fraction: f32,
    },
    Join {
        split_id: u64,
        keep: SplitSide,
    },
    SwapContent {
        first_panel_id: String,
        second_panel_id: String,
    },
    BreakSplit {
        split_id: u64,
        band: SplitSide,
        crossing_fraction: f32,
    },
    Split {
        panel_id: String,
        axis: SplitAxis,
        fraction: f32,
    },
    SplitLayout {
        side: LayoutSide,
        fraction: f32,
    },
    BeginSplit {
        axis: SplitAxis,
    },
    BeginLayoutSplit {
        side: LayoutSide,
    },
    ChangeContent {
        panel_id: String,
        content_id: String,
    },
    Panel {
        panel_id: String,
        action: PanelAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SplitSide {
    First,
    Second,
}

fn split_cursor(axis: SplitAxis) -> CursorIcon {
    match axis {
        SplitAxis::Horizontal => CursorIcon::ResizeVertical,
        SplitAxis::Vertical => CursorIcon::ResizeHorizontal,
    }
}

fn join_options(axis: SplitAxis) -> ((&'static str, SplitSide), (&'static str, SplitSide)) {
    // The label names the direction the surviving panel expands. For
    // example, Join Right keeps the left/first panel and grows it rightward.
    match axis {
        SplitAxis::Horizontal => (
            ("Join Up", SplitSide::Second),
            ("Join Down", SplitSide::First),
        ),
        SplitAxis::Vertical => (
            ("Join Left", SplitSide::Second),
            ("Join Right", SplitSide::First),
        ),
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PanelAction {
    FlipTitleBar,
    Maximize,
    RestoreMaximized,
    Close,
}

#[cfg(test)]
mod layout_tests;
