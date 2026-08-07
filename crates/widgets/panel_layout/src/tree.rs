//! Split-tree construction, queries, and mutations.
//!
//! These functions operate only on the persisted layout model. They do not
//! render widgets or retain interaction state, which keeps tree invariants
//! independent of the egui orchestration pass.

use std::collections::HashSet;

use super::contract::PanelSpec;
use super::layout::{LayoutNode, PanelState, SplitAxis, SplitSide, TitleBarPosition};

pub(crate) fn build_vertical_tree(
    panels: &[(String, f32)],
    next_id: &mut u64,
) -> Option<LayoutNode> {
    let (first, rest) = panels.split_first()?;
    let first_node = LayoutNode::Panel {
        panel: PanelState {
            id: first.0.clone(),
            content: first.0.clone(),
            title_bar_position: TitleBarPosition::Top,
        },
    };
    if rest.is_empty() {
        return Some(first_node);
    }
    let rest_weight: f32 = rest.iter().map(|(_, weight)| *weight).sum();
    let id = *next_id;
    *next_id += 1;
    Some(LayoutNode::Split {
        id,
        axis: SplitAxis::Horizontal,
        fraction: first.1 / (first.1 + rest_weight).max(0.001),
        first: Box::new(first_node),
        second: Box::new(build_vertical_tree(rest, next_id)?),
    })
}

pub(crate) fn build_panel_column(panels: &[PanelState], split_ids: &[u64]) -> LayoutNode {
    let (first, rest) = panels
        .split_first()
        .expect("a panel column must contain at least one panel");
    let first = LayoutNode::Panel {
        panel: first.clone(),
    };
    if rest.is_empty() {
        return first;
    }
    LayoutNode::Split {
        id: split_ids[0],
        axis: SplitAxis::Horizontal,
        fraction: 1.0 / panels.len() as f32,
        first: Box::new(first),
        second: Box::new(build_panel_column(rest, &split_ids[1..])),
    }
}

pub(crate) fn max_layout_id(node: Option<&LayoutNode>) -> u64 {
    match node {
        Some(LayoutNode::Panel { panel }) => panel
            .id
            .strip_prefix("panel-")
            .and_then(|number| number.parse().ok())
            .unwrap_or_default(),
        Some(LayoutNode::Split {
            id, first, second, ..
        }) => (*id)
            .max(max_layout_id(Some(first)))
            .max(max_layout_id(Some(second))),
        None => 0,
    }
}

pub(crate) fn contains_content(node: &LayoutNode, content: &str) -> bool {
    match node {
        LayoutNode::Panel { panel } => panel.content == content,
        LayoutNode::Split { first, second, .. } => {
            contains_content(first, content) || contains_content(second, content)
        }
    }
}

pub(crate) fn find_panel<'a>(node: Option<&'a LayoutNode>, id: &str) -> Option<&'a PanelState> {
    match node? {
        LayoutNode::Panel { panel } => (panel.id == id).then_some(panel),
        LayoutNode::Split { first, second, .. } => {
            find_panel(Some(first), id).or_else(|| find_panel(Some(second), id))
        }
    }
}

pub(crate) fn find_panel_mut<'a>(
    node: Option<&'a mut LayoutNode>,
    id: &str,
) -> Option<&'a mut PanelState> {
    match node? {
        LayoutNode::Panel { panel } => (panel.id == id).then_some(panel),
        LayoutNode::Split { first, second, .. } => {
            find_panel_mut(Some(first), id).or_else(|| find_panel_mut(Some(second), id))
        }
    }
}

pub(crate) fn find_panel_by_content<'a>(
    node: Option<&'a LayoutNode>,
    content: &str,
) -> Option<&'a PanelState> {
    match node? {
        LayoutNode::Panel { panel } => (panel.content == content).then_some(panel),
        LayoutNode::Split { first, second, .. } => find_panel_by_content(Some(first), content)
            .or_else(|| find_panel_by_content(Some(second), content)),
    }
}

pub(crate) fn visit_panels_mut(
    node: Option<&mut LayoutNode>,
    visitor: &mut impl FnMut(&mut PanelState),
) {
    match node {
        Some(LayoutNode::Panel { panel }) => visitor(panel),
        Some(LayoutNode::Split { first, second, .. }) => {
            visit_panels_mut(Some(first), visitor);
            visit_panels_mut(Some(second), visitor);
        }
        None => {}
    }
}

pub(crate) fn all_panels(node: Option<&LayoutNode>) -> Vec<&PanelState> {
    let mut result = Vec::new();
    fn collect<'a>(node: Option<&'a LayoutNode>, result: &mut Vec<&'a PanelState>) {
        match node {
            Some(LayoutNode::Panel { panel }) => result.push(panel),
            Some(LayoutNode::Split { first, second, .. }) => {
                collect(Some(first), result);
                collect(Some(second), result);
            }
            None => {}
        }
    }
    collect(node, &mut result);
    result
}

pub(crate) fn subtree_contains_only(node: &LayoutNode, contents: &[&str]) -> bool {
    all_panels(Some(node))
        .into_iter()
        .all(|panel| contents.contains(&panel.content.as_str()))
}

pub(crate) fn assigned_singletons(
    node: Option<&LayoutNode>,
    specs: &[PanelSpec<'_>],
) -> HashSet<String> {
    all_panels(node)
        .into_iter()
        .filter(|panel| {
            specs
                .iter()
                .any(|spec| spec.id == panel.content && spec.singleton)
        })
        .map(|panel| panel.content.clone())
        .collect()
}

pub(crate) fn available_content<'a>(
    specs: &'a [PanelSpec<'a>],
    assigned_singletons: &HashSet<String>,
) -> Option<&'a str> {
    specs
        .iter()
        .find(|spec| !spec.singleton || !assigned_singletons.contains(spec.id))
        .map(|spec| spec.id)
}

pub(crate) fn set_split_fraction(
    node: Option<&mut LayoutNode>,
    split_id: u64,
    fraction: f32,
) -> bool {
    match node {
        Some(LayoutNode::Split {
            id,
            fraction: current,
            first,
            second,
            ..
        }) => {
            if *id == split_id {
                *current = fraction.clamp(0.0, 1.0);
                true
            } else {
                set_split_fraction(Some(first), split_id, fraction)
                    || set_split_fraction(Some(second), split_id, fraction)
            }
        }
        _ => false,
    }
}

pub(crate) fn swap_panel_contents(
    mut node: Option<&mut LayoutNode>,
    first_id: &str,
    second_id: &str,
) -> bool {
    if first_id == second_id {
        return false;
    }
    let Some(first_content) =
        find_panel(node.as_deref(), first_id).map(|panel| panel.content.clone())
    else {
        return false;
    };
    let Some(second_content) =
        find_panel(node.as_deref(), second_id).map(|panel| panel.content.clone())
    else {
        return false;
    };
    let Some(first) = find_panel_mut(node.as_deref_mut(), first_id) else {
        return false;
    };
    first.content = second_content;
    let Some(second) = find_panel_mut(node, second_id) else {
        return false;
    };
    second.content = first_content;
    true
}

pub(crate) fn break_split(
    node: Option<&mut LayoutNode>,
    split_id: u64,
    dragged_band: SplitSide,
    crossing_fraction: f32,
) -> bool {
    let Some(node) = node else {
        return false;
    };
    if !matches!(node, LayoutNode::Split { id, .. } if *id == split_id) {
        return match node {
            LayoutNode::Split { first, second, .. } => {
                break_split(Some(first), split_id, dragged_band, crossing_fraction)
                    || break_split(Some(second), split_id, dragged_band, crossing_fraction)
            }
            LayoutNode::Panel { .. } => false,
        };
    }
    let snapshot = node.clone();
    match snapshot {
        LayoutNode::Split {
            id,
            axis: outer_axis,
            fraction: outer_fraction,
            first,
            second,
        } if id == split_id => {
            let LayoutNode::Split {
                id: first_child_id,
                axis: inner_axis,
                first: first_first,
                second: first_second,
                ..
            } = *first
            else {
                return false;
            };
            let LayoutNode::Split {
                id: second_child_id,
                axis: second_axis,
                first: second_first,
                second: second_second,
                ..
            } = *second
            else {
                return false;
            };
            if inner_axis == outer_axis || second_axis != inner_axis {
                return false;
            }
            let (first_band_id, second_band_id) = match dragged_band {
                SplitSide::First => (id, second_child_id),
                SplitSide::Second => (second_child_id, id),
            };
            *node = LayoutNode::Split {
                id: first_child_id,
                axis: inner_axis,
                fraction: crossing_fraction.clamp(0.1, 0.9),
                first: Box::new(LayoutNode::Split {
                    id: first_band_id,
                    axis: outer_axis,
                    fraction: outer_fraction,
                    first: first_first,
                    second: second_first,
                }),
                second: Box::new(LayoutNode::Split {
                    id: second_band_id,
                    axis: outer_axis,
                    fraction: outer_fraction,
                    first: first_second,
                    second: second_second,
                }),
            };
            true
        }
        LayoutNode::Split { .. } | LayoutNode::Panel { .. } => false,
    }
}

pub(crate) fn join_split(node: Option<&mut LayoutNode>, split_id: u64, keep: SplitSide) -> bool {
    let Some(node) = node else {
        return false;
    };
    match node {
        LayoutNode::Split {
            id, first, second, ..
        } if *id == split_id => {
            *node = match keep {
                SplitSide::First => (**first).clone(),
                SplitSide::Second => (**second).clone(),
            };
            true
        }
        LayoutNode::Split { first, second, .. } => {
            join_split(Some(first), split_id, keep) || join_split(Some(second), split_id, keep)
        }
        LayoutNode::Panel { .. } => false,
    }
}

pub(crate) fn remove_panel(node: Option<&mut LayoutNode>, panel_id: &str) -> bool {
    let Some(node) = node else {
        return false;
    };
    match node {
        LayoutNode::Split { first, second, .. } if matches!(first.as_ref(), LayoutNode::Panel { panel } if panel.id == panel_id) =>
        {
            *node = (**second).clone();
            true
        }
        LayoutNode::Split { first, second, .. } if matches!(second.as_ref(), LayoutNode::Panel { panel } if panel.id == panel_id) =>
        {
            *node = (**first).clone();
            true
        }
        LayoutNode::Split { first, second, .. } => {
            remove_panel(Some(first), panel_id) || remove_panel(Some(second), panel_id)
        }
        LayoutNode::Panel { .. } => false,
    }
}

pub(crate) fn split_content_by_content(
    node: Option<&mut LayoutNode>,
    content: &str,
    axis: SplitAxis,
    content_first: bool,
    fraction: f32,
    split_id: u64,
    new_panel: PanelState,
) -> bool {
    let Some(node) = node else {
        return false;
    };
    match node {
        LayoutNode::Panel { panel } if panel.content == content => {
            let existing = std::mem::replace(
                node,
                LayoutNode::Panel {
                    panel: new_panel.clone(),
                },
            );
            let new = LayoutNode::Panel { panel: new_panel };
            let (first, second) = if content_first {
                (new, existing)
            } else {
                (existing, new)
            };
            *node = LayoutNode::Split {
                id: split_id,
                axis,
                fraction,
                first: Box::new(first),
                second: Box::new(second),
            };
            true
        }
        LayoutNode::Split { first, second, .. } => {
            split_content_by_content(
                Some(first),
                content,
                axis,
                content_first,
                fraction,
                split_id,
                new_panel.clone(),
            ) || split_content_by_content(
                Some(second),
                content,
                axis,
                content_first,
                fraction,
                split_id,
                new_panel,
            )
        }
        LayoutNode::Panel { .. } => false,
    }
}

pub(crate) fn split_panel(
    node: Option<&mut LayoutNode>,
    panel_id: &str,
    axis: SplitAxis,
    fraction: f32,
    split_id: u64,
    new_panel: PanelState,
) -> bool {
    let Some(node) = node else {
        return false;
    };
    match node {
        LayoutNode::Panel { panel } if panel.id == panel_id => {
            let existing = LayoutNode::Panel {
                panel: panel.clone(),
            };
            *node = LayoutNode::Split {
                id: split_id,
                axis,
                fraction: fraction.clamp(0.1, 0.9),
                first: Box::new(existing),
                second: Box::new(LayoutNode::Panel { panel: new_panel }),
            };
            true
        }
        LayoutNode::Split { first, second, .. } => {
            split_panel(
                Some(first),
                panel_id,
                axis,
                fraction,
                split_id,
                new_panel.clone(),
            ) || split_panel(Some(second), panel_id, axis, fraction, split_id, new_panel)
        }
        LayoutNode::Panel { .. } => false,
    }
}
