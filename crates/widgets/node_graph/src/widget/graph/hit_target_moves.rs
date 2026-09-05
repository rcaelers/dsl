//! Conservative z-order move elision; registration and focus refresh stay intact.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use egui::Rect;

use super::layout::GraphWidgetLayout;
use crate::model::{NodeId, SocketId};

pub(crate) struct HitTargetMoves {
    isolated: HashMap<NodeId, HashSet<SocketId>>,
}

fn same_target_ids<K: Eq + Hash, V>(initial: &HashMap<K, V>, current: &HashMap<K, V>) -> bool {
    initial.len() == current.len() && initial.keys().all(|id| current.contains_key(id))
}

// Targets arrive in the desired final order. A target only needs moving if
// an earlier, potentially competing target currently ranks above it. Later
// targets restore their own order in turn. Separated rectangles cannot share
// a direct or near hit, so their relative ranks need not match.
fn moves_for_order(targets: &[(Rect, usize)], radius: f32) -> Vec<bool> {
    let mut ranks: Vec<_> = targets.iter().map(|(_, rank)| *rank).collect();
    let mut next = ranks.iter().copied().max().unwrap_or(0) + 1;
    let mut moves = vec![false; targets.len()];
    for i in 0..targets.len() {
        if (0..i).any(|j| {
            ranks[j] > ranks[i] && targets[j].0.expand(radius * 2.0).intersects(targets[i].0)
        }) {
            moves[i] = true;
            ranks[i] = next;
            next += 1;
        }
    }
    moves
}

impl HitTargetMoves {
    pub(crate) fn new(
        ui: &egui::Ui,
        zoom: f32,
        initial: &GraphWidgetLayout,
        current: &GraphWidgetLayout,
    ) -> Self {
        let mut result = Self {
            isolated: HashMap::new(),
        };
        // Inline controls register between nodes at higher zoom. Transformed
        // layers and invalid hit radii use the ordinary raising path.
        let radius = ui.ctx().global_style().interaction.interact_radius;
        if !(0.0..0.6).contains(&zoom)
            || !radius.is_finite()
            || radius < 0.0
            || ui.ctx().layer_transform_to_global(ui.layer_id()).is_some()
        {
            return result;
        }
        // Removed targets remain registered for this pass even though they no
        // longer appear in the drawing layout. Do not leave another node below
        // those stale targets. Equal counts alone miss remove-and-add edits.
        if !same_target_ids(&initial.node_screen_rects, &current.node_screen_rects)
            || !same_target_ids(&initial.header_screen_rects, &current.header_screen_rects)
            || !same_target_ids(
                &initial.collapse_toggle_screen_rects,
                &current.collapse_toggle_screen_rects,
            )
            || !same_target_ids(&initial.socket_hit_rects, &current.socket_hit_rects)
        {
            return result;
        }
        let mut bounds = current.node_screen_rects.clone();
        for (&id, &rect) in current
            .header_screen_rects
            .iter()
            .chain(&current.collapse_toggle_screen_rects)
        {
            bounds
                .entry(id)
                .and_modify(|bound| *bound = bound.union(rect))
                .or_insert(rect);
        }
        for (&socket, &rect) in &current.socket_hit_rects {
            bounds
                .entry(socket.node)
                .and_modify(|bound| *bound = bound.union(rect))
                .or_insert(rect);
        }
        if bounds
            .values()
            .any(|bound| !bound.is_finite() || bound.is_negative())
        {
            return result;
        }
        let initial_order: HashMap<_, _> = initial
            .socket_hit_rects
            .keys()
            .enumerate()
            .map(|(rank, &socket)| (socket, rank + 2))
            .collect();
        for (&node, &bound) in &bounds {
            if !ui.clip_rect().contains_rect(bound.expand(radius * 2.0))
                || bounds.iter().any(|(&other, &rect)| {
                    other != node && bound.expand(radius * 2.0).intersects(rect)
                })
            {
                continue;
            }
            let (Some(&body), Some(&header), Some(&toggle), Some(sockets)) = (
                current.node_screen_rects.get(&node),
                current.header_screen_rects.get(&node),
                current.collapse_toggle_screen_rects.get(&node),
                current.socket_hit_order_by_node.get(&node),
            ) else {
                continue;
            };
            // Geometry/topology changes between input allocation and drawing
            // always refresh using the original full raising behavior.
            if initial.node_screen_rects.get(&node) != Some(&body)
                || initial.header_screen_rects.get(&node) != Some(&header)
                || initial.collapse_toggle_screen_rects.get(&node) != Some(&toggle)
                || initial.socket_hit_order_by_node.get(&node).map(Vec::len) != Some(sockets.len())
                || sockets.iter().any(|socket| {
                    !initial_order.contains_key(socket)
                        || initial.socket_hit_rects.get(socket)
                            != current.socket_hit_rects.get(socket)
                })
            {
                continue;
            }
            let mut targets = vec![(body, 0), (header, 1), (toggle, initial_order.len() + 2)];
            targets.extend(
                sockets
                    .iter()
                    .map(|socket| (current.socket_hit_rects[socket], initial_order[socket])),
            );
            let moves = moves_for_order(&targets, radius);
            debug_assert!(moves[..3].iter().all(|moved| !moved));
            result.isolated.insert(
                node,
                sockets
                    .iter()
                    .zip(&moves[3..])
                    .filter_map(|(&socket, &moved)| (!moved).then_some(socket))
                    .collect(),
            );
        }
        result
    }

    pub(crate) fn base_move_to_top(&self, node: NodeId) -> bool {
        !self.isolated.contains_key(&node)
    }

    pub(crate) fn socket_move_to_top(&self, socket: SocketId) -> bool {
        !self
            .isolated
            .get(&socket.node)
            .is_some_and(|skipped| skipped.contains(&socket))
    }
}

#[cfg(test)]
mod hit_target_moves_tests {
    use egui::{Pos2, Vec2};

    use super::*;

    #[test]
    fn elision_preserves_every_competing_pair_for_all_small_orders() {
        fn permutations(values: &mut [usize], index: usize, visit: &mut impl FnMut(&[usize])) {
            if index == values.len() {
                visit(values);
                return;
            }
            for i in index..values.len() {
                values.swap(index, i);
                permutations(values, index + 1, visit);
                values.swap(index, i);
            }
        }
        for radius in [0.0, 5.0] {
            permutations(&mut [0, 1, 2, 3, 4], 0, &mut |ranks| {
                let targets: Vec<_> = ranks
                    .iter()
                    .enumerate()
                    .map(|(i, &rank)| {
                        (
                            Rect::from_min_size(Pos2::new(i as f32 * 12.0, 0.0), Vec2::splat(10.0)),
                            rank,
                        )
                    })
                    .collect();
                let moves = moves_for_order(&targets, radius);
                let mut final_ranks = ranks.to_vec();
                let mut next = ranks.len();
                for (i, moved) in moves.into_iter().enumerate() {
                    if moved {
                        final_ranks[i] = next;
                        next += 1;
                    }
                }
                for i in 0..targets.len() {
                    for j in 0..i {
                        if targets[j].0.expand(radius * 2.0).intersects(targets[i].0) {
                            assert!(final_ranks[j] < final_ranks[i]);
                        }
                    }
                }
            });
        }
    }
}
