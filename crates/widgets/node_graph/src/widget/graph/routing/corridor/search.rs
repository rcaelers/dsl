use std::cmp::Ordering;
use std::collections::BinaryHeap;

use egui::{Pos2, Rect};

use super::super::{RouteConfig, RouteFailure, WorkBudget};
use super::obstacle::clear;

/// Columns at obstacle boundaries partition the plane into slabs. Retaining each
/// opening's Y coordinates (rather than one cost per interval) preserves entry position.
/// The same visibility lattice supports monotonic and bidirectional search.
pub(crate) struct Channels {
    xs: Vec<f32>,
    ys: Vec<f32>,
}

impl Channels {
    pub(crate) fn new(
        start: Pos2,
        end: Pos2,
        obstacles: &[Rect],
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
        Ok(Self { xs, ys })
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
            if vertex == goal {
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
                let a = self.point(vertex);
                let b = self.point(next);
                if monotonic && (b.x < start.x || b.x > end.x) {
                    continue;
                }
                if !clear([a, b], obstacles, None, budget)? {
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
