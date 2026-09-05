//! Monotonic shared-lane search with a conservative swept rectangular footprint.

use egui::{Pos2, Rect};

use super::contract::{RouteConfig, RouteFailure, WorkBudget};
use super::corridor::Channels;

pub(crate) fn route_lanes(
    entry: Pos2,
    exit: Pos2,
    obstacles: &[Rect],
    count: usize,
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> Result<Vec<Vec<Pos2>>, RouteFailure> {
    if count < 2 || !config.lane_spacing.is_finite() || config.lane_spacing <= 0.0 {
        return Err(RouteFailure::InvalidGeometry);
    }
    budget.spend(count)?;
    let height = (count - 1) as f32 * config.lane_spacing;
    let start = Pos2::new(entry.x + height, entry.y);
    let end = Pos2::new(exit.x - height, exit.y);
    if !start.is_finite() || !end.is_finite() || start.x >= end.x {
        return Err(RouteFailure::NoCorridor);
    }
    budget.spend(obstacles.len())?;
    // A top-lane reference carries [0,height] in Y. Staggered vertical runs
    // shift by up to height in either X direction. Inflating obstacles by the
    // reflected footprint checks every slab and connecting opening continuously.
    let swept: Vec<_> = obstacles
        .iter()
        .map(|r| {
            Rect::from_min_max(
                Pos2::new(
                    (r.min.x - height).next_down(),
                    (r.min.y - height).next_down(),
                ),
                Pos2::new((r.max.x + height).next_up(), r.max.y),
            )
        })
        .collect();
    if swept
        .iter()
        .any(|r| !r.min.is_finite() || !r.max.is_finite())
    {
        return Err(RouteFailure::InvalidGeometry);
    }
    for (a, b) in [(entry, start), (end, exit)] {
        let envelope = Rect::from_min_max(a, Pos2::new(b.x, b.y + height));
        if !envelope.min.is_finite() || !envelope.max.is_finite() {
            return Err(RouteFailure::InvalidGeometry);
        }
        for obstacle in obstacles {
            budget.spend(1)?;
            if envelope.intersects(*obstacle) {
                return Err(RouteFailure::NoCorridor);
            }
        }
    }
    let channels = Channels::new(start, end, &swept, config, budget)?;
    let spine = channels.find(start, end, &swept, config, true, budget)?;
    let mut compact: Vec<Pos2> = Vec::new();
    for point in spine {
        if compact.len() >= 2 {
            let a = compact[compact.len() - 2];
            let b = compact[compact.len() - 1];
            if (a.x == b.x && b.x == point.x) || (a.y == b.y && b.y == point.y) {
                compact.pop();
            }
        }
        compact.push(point);
    }
    let mut lanes = Vec::with_capacity(count);
    for i in 0..count {
        budget.spend(compact.len())?;
        let offset = i as f32 * config.lane_spacing;
        let mut points = vec![Pos2::new(entry.x, entry.y + offset)];
        for pair in compact.windows(2) {
            if pair[0].x == pair[1].x {
                let x = pair[0].x
                    + if pair[1].y > pair[0].y {
                        -offset
                    } else {
                        offset
                    };
                points.extend([
                    Pos2::new(x, pair[0].y + offset),
                    Pos2::new(x, pair[1].y + offset),
                ]);
            }
        }
        points.push(Pos2::new(exit.x, exit.y + offset));
        // A narrow horizontal run between opposite turns can fold an offset lane.
        // Reject it rather than reversing lane order or accepting an X backtrack.
        if points.iter().any(|p| !p.is_finite()) || points.windows(2).any(|p| p[0].x > p[1].x) {
            return Err(RouteFailure::NoCorridor);
        }
        lanes.push(points);
    }
    // Check representable spacing at every reference height, not only the endpoints.
    for point in compact {
        for i in 1..count {
            budget.spend(1)?;
            let a = point.y + (i - 1) as f32 * config.lane_spacing;
            let b = point.y + i as f32 * config.lane_spacing;
            if b as f64 - (a as f64) < config.lane_spacing as f64 {
                return Err(RouteFailure::NoCorridor);
            }
            for sign in [-1.0, 1.0] {
                let a = point.x + sign * (i - 1) as f32 * config.lane_spacing;
                let b = point.x + sign * i as f32 * config.lane_spacing;
                if (b as f64 - a as f64).abs() < config.lane_spacing as f64 {
                    return Err(RouteFailure::NoCorridor);
                }
            }
        }
    }
    Ok(lanes)
}
