use std::cmp::Ordering;
use std::collections::BinaryHeap;

use egui::{Pos2, Rect};

use super::super::{PortSide, RouteConfig, RouteFailure, WorkBudget};
use super::obstacle::clear;

/// Columns at obstacle boundaries partition the plane into slabs. Retaining each
/// opening's Y coordinates (rather than one cost per interval) preserves entry position.
/// The same visibility lattice supports monotonic and bidirectional search.
pub(crate) struct Channels {
    xs: Vec<f32>,
    ys: Vec<f32>,
    reserved: Vec<[Pos2; 2]>,
    spacing: f32,
    endpoint_sides: Option<(PortSide, PortSide)>,
}

impl Channels {
    pub(crate) fn new(
        start: Pos2,
        end: Pos2,
        obstacles: &[Rect],
        config: &RouteConfig,
        budget: &mut WorkBudget,
    ) -> Result<Self, RouteFailure> {
        Self::avoiding_runs(start, end, obstacles, &[], config, budget)
    }

    /// Parallel runs are reserved, but perpendicular crossings remain available.
    pub(crate) fn avoiding_runs(
        start: Pos2,
        end: Pos2,
        obstacles: &[Rect],
        reserved: &[[Pos2; 2]],
        config: &RouteConfig,
        budget: &mut WorkBudget,
    ) -> Result<Self, RouteFailure> {
        budget.spend(obstacles.len())?;
        let mut xs = vec![start.x, end.x];
        let mut ys = vec![start.y, end.y];
        for rect in obstacles {
            xs.extend([
                (rect.min.x - config.safety).next_down(),
                (rect.max.x + config.safety).next_up(),
            ]);
            ys.extend([
                (rect.min.y - config.safety).next_down(),
                (rect.max.y + config.safety).next_up(),
            ]);
        }
        budget.spend(reserved.len())?;
        for &[a, b] in reserved {
            if a.x == b.x {
                xs.extend([
                    (a.x - config.lane_spacing).next_down(),
                    (a.x + config.lane_spacing).next_up(),
                ]);
            } else if a.y == b.y {
                ys.extend([
                    (a.y - config.lane_spacing).next_down(),
                    (a.y + config.lane_spacing).next_up(),
                ]);
            }
        }
        // Outermost coordinates already form a finite envelope outside every obstacle.
        for axis in [&mut xs, &mut ys] {
            if !axis.iter().all(|x| x.is_finite()) {
                return Err(RouteFailure::InvalidGeometry);
            }
            budget.spend(axis.len())?;
            axis.sort_by(f32::total_cmp);
            axis.dedup_by(|a, b| *a == *b);
        }
        let count = xs
            .len()
            .checked_mul(ys.len())
            .ok_or(RouteFailure::WorkLimit)?;
        if count > config.max_vertices || count.checked_mul(5).is_none() {
            return Err(RouteFailure::WorkLimit);
        }
        budget.spend(count)?;
        Ok(Self {
            xs,
            ys,
            reserved: reserved.to_vec(),
            spacing: config.lane_spacing,
            endpoint_sides: None,
        })
    }

    /// Join mandatory horizontal escapes without reversing along either escape.
    pub(crate) fn with_endpoint_sides(mut self, source: PortSide, target: PortSide) -> Self {
        self.endpoint_sides = Some((source, target));
        self
    }

    fn point(&self, vertex: usize) -> Pos2 {
        Pos2::new(
            self.xs[vertex % self.xs.len()],
            self.ys[vertex / self.xs.len()],
        )
    }

    fn vertex(&self, point: Pos2) -> usize {
        let x = self
            .xs
            .iter()
            .position(|value| *value == point.x)
            .expect("endpoint coordinate");
        let y = self
            .ys
            .iter()
            .position(|value| *value == point.y)
            .expect("endpoint coordinate");
        y * self.xs.len() + x
    }

    pub(crate) fn find(
        &self,
        start: Pos2,
        end: Pos2,
        obstacles: &[Rect],
        config: &RouteConfig,
        monotonic: bool,
        budget: &mut WorkBudget,
    ) -> Result<Vec<Pos2>, RouteFailure> {
        let states = self.xs.len() * self.ys.len() * 5;
        budget.spend(states)?;
        let mut cost = vec![f64::INFINITY; states];
        let mut previous = vec![usize::MAX; states];
        let start_state = self.vertex(start) * 5 + 4;
        let goal = self.vertex(end);
        cost[start_state] = 0.0;
        let mut queue = BinaryHeap::from([Entry {
            cost: 0.0,
            state: start_state,
        }]);
        while let Some(entry) = queue.pop() {
            budget.spend(1)?;
            if entry.cost != cost[entry.state] {
                continue;
            }
            let vertex = entry.state / 5;
            // Coincident escapes still need a corridor when both ports face the
            // same side: directly joining them would reverse at the shared point.
            let direct_reversal = entry.state == start_state
                && self
                    .endpoint_sides
                    .is_some_and(|(source, target)| source == target);
            if vertex == goal && !direct_reversal {
                let mut route = Vec::new();
                let mut state = entry.state;
                loop {
                    budget.spend(1)?;
                    route.push(self.point(state / 5));
                    if state == start_state {
                        break;
                    }
                    state = previous[state];
                }
                route.reverse();
                return Ok(route);
            }
            let x = vertex % self.xs.len();
            let y = vertex / self.xs.len();
            // Stable neighbor order is also the tie breaker for equal-cost paths.
            let neighbors = [
                (x + 1 < self.xs.len()).then_some(vertex + 1),
                (!monotonic && x > 0).then(|| vertex - 1),
                (y + 1 < self.ys.len()).then_some(vertex + self.xs.len()),
                (y > 0).then(|| vertex - self.xs.len()),
            ];
            for (direction, next) in neighbors.into_iter().enumerate() {
                let Some(next) = next else {
                    continue;
                };
                let incoming = entry.state % 5;
                if incoming != 4 && direction == (incoming ^ 1) {
                    continue;
                }
                if let Some((source, target)) = self.endpoint_sides {
                    let reverses_source = entry.state == start_state
                        && matches!(
                            (source, direction),
                            (PortSide::Right, 1) | (PortSide::Left, 0)
                        );
                    let reverses_target = next == goal
                        && matches!(
                            (target, direction),
                            (PortSide::Left, 1) | (PortSide::Right, 0)
                        );
                    if reverses_source || reverses_target {
                        continue;
                    }
                }
                let a = self.point(vertex);
                let b = self.point(next);
                if monotonic && (b.x < start.x || b.x > end.x) {
                    continue;
                }
                if !clear([a, b], obstacles, None, budget)? {
                    continue;
                }
                if !self.run_clear([a, b], budget)? {
                    continue;
                }
                let distance = (b.x as f64 - a.x as f64).abs()
                    + (b.y as f64 - a.y as f64).abs() * config.vertical_weight;
                let bend = if entry.state % 5 != 4 && entry.state % 5 != direction {
                    config.bend_cost
                } else {
                    0.0
                };
                let next_cost = entry.cost + distance + bend;
                if !next_cost.is_finite() {
                    return Err(RouteFailure::InvalidGeometry);
                }
                let next_state = next * 5 + direction;
                if next_cost < cost[next_state] {
                    cost[next_state] = next_cost;
                    previous[next_state] = entry.state;
                    queue.push(Entry {
                        cost: next_cost,
                        state: next_state,
                    });
                }
            }
        }
        Err(RouteFailure::NoCorridor)
    }

    pub(crate) fn run_clear(
        &self,
        segment: [Pos2; 2],
        budget: &mut WorkBudget,
    ) -> Result<bool, RouteFailure> {
        for &other in &self.reserved {
            budget.spend(1)?;
            if parallel_overlap(segment, other, self.spacing) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Positive-length parallel overlap, not a perpendicular crossing or endpoint touch.
pub(crate) fn parallel_overlap(a: [Pos2; 2], b: [Pos2; 2], spacing: f32) -> bool {
    let overlap = |a: f32, b: f32, c: f32, d: f32| a.min(b).max(c.min(d)) < a.max(b).min(c.max(d));
    let near = |a: f32, b: f32| a == b || (a as f64 - b as f64).abs() < spacing as f64;
    (a[0].x == a[1].x
        && b[0].x == b[1].x
        && near(a[0].x, b[0].x)
        && overlap(a[0].y, a[1].y, b[0].y, b[1].y))
        || (a[0].y == a[1].y
            && b[0].y == b[1].y
            && near(a[0].y, b[0].y)
            && overlap(a[0].x, a[1].x, b[0].x, b[1].x))
}

#[derive(Clone, Copy, PartialEq)]
struct Entry {
    cost: f64,
    state: usize,
}
impl Eq for Entry {}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.state.cmp(&self.state))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
