//! Pure split-tree geometry and boundary calculations.
//!
//! This module turns a layout tree into panel and splitter rectangles and
//! computes pointer-driven boundary changes. It does not own persisted state,
//! host synchronization, or egui interaction state.

use egui::{Color32, Rect, Stroke, Ui};

use super::contract::{PanelGeometry, PanelSpec};
use super::layout::{
    LayoutAction, LayoutNode, PanelLayoutStyle, PanelState, SplitAxis, TitleBarPosition,
};
use super::tree::contains_content;

const BOUNDARY_SNAP_GRID: f32 = 16.0;
const BOUNDARY_ALIGNMENT_DISTANCE: f32 = 10.0;
pub(crate) const BOUNDARY_EXTEND_DISTANCE: f32 = 12.0;

#[derive(Debug, Clone)]
pub(crate) struct BoundaryGeometry {
    pub(crate) id: u64,
    pub(crate) axis: SplitAxis,
    pub(crate) rect: Rect,
    pub(crate) parent_rect: Rect,
}

impl BoundaryGeometry {
    fn fraction_at(&self, pointer: egui::Pos2) -> f32 {
        self.fraction_for_coordinate(axis_coordinate(self.axis, pointer))
    }

    pub(crate) fn snapped_fraction_at(
        &self,
        pointer: egui::Pos2,
        boundaries: &[BoundaryGeometry],
        root_rect: Rect,
    ) -> f32 {
        self.snapped_fraction_at_excluding(pointer, boundaries, root_rect, None)
    }

    fn snapped_fraction_at_excluding(
        &self,
        pointer: egui::Pos2,
        boundaries: &[BoundaryGeometry],
        root_rect: Rect,
        excluded_boundary: Option<u64>,
    ) -> f32 {
        self.fraction_for_coordinate(self.snapped_coordinate_at(
            pointer,
            boundaries,
            root_rect,
            excluded_boundary,
        ))
    }

    fn snapped_coordinate_at(
        &self,
        pointer: egui::Pos2,
        boundaries: &[BoundaryGeometry],
        root_rect: Rect,
        excluded_boundary: Option<u64>,
    ) -> f32 {
        let coordinate = axis_coordinate(self.axis, pointer);
        self.nearest_parallel_boundary_to_excluding(
            boundaries,
            BOUNDARY_ALIGNMENT_DISTANCE,
            coordinate,
            excluded_boundary,
        )
        .map(BoundaryGeometry::coordinate)
        .unwrap_or_else(|| {
            let origin = axis_min(self.axis, root_rect);
            origin + ((coordinate - origin) / BOUNDARY_SNAP_GRID).round() * BOUNDARY_SNAP_GRID
        })
    }

    pub(crate) fn nearest_parallel_boundary<'a>(
        &self,
        boundaries: &'a [BoundaryGeometry],
        maximum_distance: f32,
    ) -> Option<&'a BoundaryGeometry> {
        self.nearest_parallel_boundary_to_excluding(
            boundaries,
            maximum_distance,
            self.coordinate(),
            None,
        )
    }

    fn nearest_parallel_boundary_to_excluding<'a>(
        &self,
        boundaries: &'a [BoundaryGeometry],
        maximum_distance: f32,
        coordinate: f32,
        excluded_boundary: Option<u64>,
    ) -> Option<&'a BoundaryGeometry> {
        boundaries
            .iter()
            .filter(|candidate| {
                candidate.id != self.id
                    && Some(candidate.id) != excluded_boundary
                    && candidate.axis == self.axis
            })
            .min_by(|left, right| {
                (coordinate - left.coordinate())
                    .abs()
                    .total_cmp(&(coordinate - right.coordinate()).abs())
            })
            .filter(|candidate| (coordinate - candidate.coordinate()).abs() <= maximum_distance)
    }

    pub(crate) fn resize_actions(
        &self,
        pointer: egui::Pos2,
        boundaries: &[BoundaryGeometry],
        root_rect: Rect,
        snap: bool,
        extend: bool,
    ) -> Vec<LayoutAction> {
        let parallel = extend
            .then(|| self.nearest_parallel_boundary(boundaries, BOUNDARY_EXTEND_DISTANCE))
            .flatten();
        let target_fraction = if snap {
            if let Some(parallel) = parallel {
                self.snapped_fraction_at_excluding(
                    pointer,
                    boundaries,
                    root_rect,
                    Some(parallel.id),
                )
            } else {
                self.snapped_fraction_at(pointer, boundaries, root_rect)
            }
        } else {
            self.fraction_at(pointer)
        };
        let mut actions = vec![LayoutAction::SetFraction {
            split_id: self.id,
            fraction: target_fraction,
        }];
        if let Some(parallel) = parallel {
            let coordinate = self.coordinate_for_fraction(target_fraction);
            actions.push(LayoutAction::SetFraction {
                split_id: parallel.id,
                fraction: parallel.fraction_for_coordinate(coordinate),
            });
        }
        actions
    }

    fn coordinate(&self) -> f32 {
        axis_coordinate(self.axis, self.rect.center())
    }

    fn fraction_for_coordinate(&self, coordinate: f32) -> f32 {
        let parent_extent = axis_extent(self.axis, self.parent_rect);
        let splitter_extent = axis_extent(self.axis, self.rect);
        let usable = (parent_extent - splitter_extent).max(1.0);
        ((coordinate - axis_min(self.axis, self.parent_rect) - splitter_extent * 0.5) / usable)
            .clamp(0.1, 0.9)
    }

    pub(crate) fn coordinate_for_fraction(&self, fraction: f32) -> f32 {
        let parent_extent = axis_extent(self.axis, self.parent_rect);
        let splitter_extent = axis_extent(self.axis, self.rect);
        let usable = (parent_extent - splitter_extent).max(1.0);
        axis_min(self.axis, self.parent_rect)
            + splitter_extent * 0.5
            + usable * fraction.clamp(0.1, 0.9)
    }

    pub(crate) fn visual_rect(&self, thickness: f32) -> Rect {
        match self.axis {
            SplitAxis::Horizontal => {
                Rect::from_center_size(self.rect.center(), egui::vec2(self.rect.width(), thickness))
            }
            SplitAxis::Vertical => Rect::from_center_size(
                self.rect.center(),
                egui::vec2(thickness, self.rect.height()),
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BoundaryBreakCandidate {
    pub(crate) axis: SplitAxis,
    pub(crate) coordinate: f32,
    pub(crate) crossing_fraction: f32,
}

pub(crate) fn boundary_break_candidate(
    root: Option<&LayoutNode>,
    split_id: u64,
    boundaries: &[BoundaryGeometry],
) -> Option<BoundaryBreakCandidate> {
    let (axis, first_id, second_id) = breakable_child_splits(root?, split_id)?;
    let first = boundaries.iter().find(|boundary| boundary.id == first_id)?;
    let second = boundaries
        .iter()
        .find(|boundary| boundary.id == second_id)?;
    let coordinate = (first.coordinate() + second.coordinate()) * 0.5;
    if (first.coordinate() - second.coordinate()).abs() > BOUNDARY_EXTEND_DISTANCE {
        return None;
    }
    Some(BoundaryBreakCandidate {
        axis,
        coordinate,
        crossing_fraction: first.fraction_for_coordinate(coordinate),
    })
}

fn breakable_child_splits(node: &LayoutNode, split_id: u64) -> Option<(SplitAxis, u64, u64)> {
    match node {
        LayoutNode::Split {
            id,
            axis,
            first,
            second,
            ..
        } if *id == split_id => {
            let LayoutNode::Split {
                id: first_id,
                axis: first_axis,
                ..
            } = first.as_ref()
            else {
                return None;
            };
            let LayoutNode::Split {
                id: second_id,
                axis: second_axis,
                ..
            } = second.as_ref()
            else {
                return None;
            };
            (*first_axis == *second_axis && *first_axis != *axis).then_some((
                *first_axis,
                *first_id,
                *second_id,
            ))
        }
        LayoutNode::Split { first, second, .. } => breakable_child_splits(first, split_id)
            .or_else(|| breakable_child_splits(second, split_id)),
        LayoutNode::Panel { .. } => None,
    }
}

pub(crate) fn paint_extended_boundary_guide(
    ui: &Ui,
    axis: SplitAxis,
    root_rect: Rect,
    coordinate: f32,
) {
    let points = match axis {
        SplitAxis::Horizontal => [
            egui::pos2(root_rect.left(), coordinate),
            egui::pos2(root_rect.right(), coordinate),
        ],
        SplitAxis::Vertical => [
            egui::pos2(coordinate, root_rect.top()),
            egui::pos2(coordinate, root_rect.bottom()),
        ],
    };
    ui.painter()
        .line_segment(points, Stroke::new(2.0, Color32::from_rgb(185, 185, 185)));
}

pub(crate) fn axis_coordinate(axis: SplitAxis, position: egui::Pos2) -> f32 {
    match axis {
        SplitAxis::Horizontal => position.y,
        SplitAxis::Vertical => position.x,
    }
}

fn axis_min(axis: SplitAxis, rect: Rect) -> f32 {
    match axis {
        SplitAxis::Horizontal => rect.top(),
        SplitAxis::Vertical => rect.left(),
    }
}

fn axis_extent(axis: SplitAxis, rect: Rect) -> f32 {
    match axis {
        SplitAxis::Horizontal => rect.height(),
        SplitAxis::Vertical => rect.width(),
    }
}

pub(crate) fn collect_geometries(
    node: &LayoutNode,
    rect: Rect,
    specs: &[PanelSpec<'_>],
    style: &PanelLayoutStyle,
    panels: &mut Vec<PanelGeometry>,
    boundaries: &mut Vec<BoundaryGeometry>,
) {
    match node {
        LayoutNode::Panel { panel } => {
            push_panel_geometry(panel, rect, false, style, panels);
        }
        LayoutNode::Split {
            id,
            axis,
            fraction,
            first,
            second,
        } => {
            let (first_rect, splitter_rect, second_rect) = split_rects(
                rect,
                *axis,
                *fraction,
                minimum_size(first, specs, style),
                minimum_size(second, specs, style),
                style.splitter_size,
            );
            boundaries.push(BoundaryGeometry {
                id: *id,
                axis: *axis,
                rect: splitter_rect,
                parent_rect: rect,
            });
            collect_geometries(first, first_rect, specs, style, panels, boundaries);
            collect_geometries(second, second_rect, specs, style, panels, boundaries);
        }
    }
}

pub(crate) fn push_panel_geometry(
    panel: &PanelState,
    allocated_rect: Rect,
    maximized: bool,
    style: &PanelLayoutStyle,
    panels: &mut Vec<PanelGeometry>,
) {
    let title_height = style.title_height.min(allocated_rect.height());
    let title_rect = match panel.title_bar_position {
        TitleBarPosition::Top => Rect::from_min_size(
            allocated_rect.min,
            egui::vec2(allocated_rect.width(), title_height),
        ),
        TitleBarPosition::Bottom => Rect::from_min_size(
            egui::pos2(
                allocated_rect.left(),
                allocated_rect.bottom() - title_height,
            ),
            egui::vec2(allocated_rect.width(), title_height),
        ),
    };
    let radius = f32::from(style.corner_radius);
    let body_height = (allocated_rect.height() - title_height - radius).max(0.0);
    let body_min = match panel.title_bar_position {
        TitleBarPosition::Top => title_rect.left_bottom(),
        TitleBarPosition::Bottom => {
            egui::pos2(allocated_rect.left(), allocated_rect.top() + radius)
        }
    };
    panels.push(PanelGeometry {
        panel_id: panel.id.clone(),
        content_id: panel.content.clone(),
        title_rect,
        title_interaction_rect: None,
        body_rect: Rect::from_min_size(body_min, egui::vec2(allocated_rect.width(), body_height)),
        panel_rect: allocated_rect,
        allocated_rect,
        title_bar_position: panel.title_bar_position,
        maximized,
    });
}

pub(crate) fn title_interaction_rect(
    title_rect: Rect,
    content_right: f32,
    controls_left: f32,
) -> Option<Rect> {
    let left = content_right.clamp(title_rect.left(), title_rect.right());
    let right = controls_left.clamp(title_rect.left(), title_rect.right());
    (right > left).then(|| {
        Rect::from_min_max(
            egui::pos2(left, title_rect.top()),
            egui::pos2(right, title_rect.bottom()),
        )
    })
}

pub(crate) fn panel_at_pointer(
    panels: &[PanelGeometry],
    pointer: egui::Pos2,
) -> Option<&PanelGeometry> {
    panels
        .iter()
        .find(|panel| panel.panel_rect.contains(pointer))
}

pub(crate) fn adjacent_panels_at_boundary(
    panels: &[PanelGeometry],
    boundary: &BoundaryGeometry,
    pointer: egui::Pos2,
) -> Option<(String, String)> {
    let (first_point, second_point) = match boundary.axis {
        SplitAxis::Horizontal => {
            let x = pointer
                .x
                .clamp(boundary.parent_rect.left(), boundary.parent_rect.right());
            (
                egui::pos2(x, boundary.rect.top() - 0.5),
                egui::pos2(x, boundary.rect.bottom() + 0.5),
            )
        }
        SplitAxis::Vertical => {
            let y = pointer
                .y
                .clamp(boundary.parent_rect.top(), boundary.parent_rect.bottom());
            (
                egui::pos2(boundary.rect.left() - 0.5, y),
                egui::pos2(boundary.rect.right() + 0.5, y),
            )
        }
    };
    let first = panel_at_pointer(panels, first_point)?;
    let second = panel_at_pointer(panels, second_point)?;
    (first.panel_id != second.panel_id).then(|| (first.panel_id.clone(), second.panel_id.clone()))
}

pub(crate) fn split_rects(
    rect: Rect,
    axis: SplitAxis,
    fraction: f32,
    first_minimum: egui::Vec2,
    second_minimum: egui::Vec2,
    splitter_size: f32,
) -> (Rect, Rect, Rect) {
    let total = match axis {
        SplitAxis::Horizontal => rect.height(),
        SplitAxis::Vertical => rect.width(),
    };
    let usable = (total - splitter_size).max(0.0);
    let first_minimum = match axis {
        SplitAxis::Horizontal => first_minimum.y,
        SplitAxis::Vertical => first_minimum.x,
    }
    .min(usable);
    let second_minimum = match axis {
        SplitAxis::Horizontal => second_minimum.y,
        SplitAxis::Vertical => second_minimum.x,
    }
    .min(usable);
    let mut first_extent = usable * fraction.clamp(0.0, 1.0);
    if first_minimum + second_minimum <= usable {
        first_extent = first_extent.clamp(first_minimum, usable - second_minimum);
    }
    let second_extent = (usable - first_extent).max(0.0);
    match axis {
        SplitAxis::Horizontal => {
            let first = Rect::from_min_size(rect.min, egui::vec2(rect.width(), first_extent));
            let splitter =
                Rect::from_min_size(first.left_bottom(), egui::vec2(rect.width(), splitter_size));
            let second = Rect::from_min_size(
                splitter.left_bottom(),
                egui::vec2(rect.width(), second_extent),
            );
            (first, splitter, second)
        }
        SplitAxis::Vertical => {
            let first = Rect::from_min_size(rect.min, egui::vec2(first_extent, rect.height()));
            let splitter =
                Rect::from_min_size(first.right_top(), egui::vec2(splitter_size, rect.height()));
            let second = Rect::from_min_size(
                splitter.right_top(),
                egui::vec2(second_extent, rect.height()),
            );
            (first, splitter, second)
        }
    }
}

fn minimum_size(
    node: &LayoutNode,
    specs: &[PanelSpec<'_>],
    style: &PanelLayoutStyle,
) -> egui::Vec2 {
    match node {
        LayoutNode::Panel { panel } => {
            let spec = specs.iter().find(|spec| spec.id == panel.content);
            let width = spec.map_or(100.0, |spec| spec.minimum_width);
            let height = spec
                .map_or(100.0, |spec| spec.minimum_height)
                .max(style.title_height);
            egui::vec2(width, height)
        }
        LayoutNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            let first = minimum_size(first, specs, style);
            let second = minimum_size(second, specs, style);
            match axis {
                SplitAxis::Horizontal => egui::vec2(
                    first.x.max(second.x),
                    first.y + style.splitter_size + second.y,
                ),
                SplitAxis::Vertical => egui::vec2(
                    first.x + style.splitter_size + second.x,
                    first.y.max(second.y),
                ),
            }
        }
    }
}

pub(crate) fn fraction_in_rect(axis: SplitAxis, rect: Rect, pointer: egui::Pos2) -> f32 {
    let fraction = match axis {
        SplitAxis::Horizontal => (pointer.y - rect.top()) / rect.height().max(1.0),
        SplitAxis::Vertical => (pointer.x - rect.left()) / rect.width().max(1.0),
    };
    fraction.clamp(0.1, 0.9)
}

pub(crate) fn find_content_split_fraction(
    node: &LayoutNode,
    first_content: &str,
    second_content: &str,
) -> Option<f32> {
    let LayoutNode::Split {
        fraction,
        first,
        second,
        ..
    } = node
    else {
        return None;
    };
    if contains_content(first, first_content) && contains_content(second, second_content) {
        return Some(*fraction);
    }
    if contains_content(first, second_content) && contains_content(second, first_content) {
        return Some(1.0 - *fraction);
    }
    find_content_split_fraction(first, first_content, second_content)
        .or_else(|| find_content_split_fraction(second, first_content, second_content))
}
