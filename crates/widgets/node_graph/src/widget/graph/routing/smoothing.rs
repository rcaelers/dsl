//! Optional individual-path corner rounding. Bundle ordering is not inferred here.

use egui::{Pos2, Rect, Vec2};

use super::contract::{RouteConfig, RouteInput, WorkBudget};
use super::corridor::{cubic_clear, escape, expanded};
use super::geometry::{PathSegment, WirePath};

pub(crate) fn smooth_route(
    input: RouteInput<'_>,
    path: WirePath,
    config: &RouteConfig,
    zoom: f32,
    budget: &mut WorkBudget,
) -> WirePath {
    let tolerance = 0.5 / zoom;
    if !config.corner_radius.is_finite()
        || config.corner_radius <= 0.0
        || !zoom.is_finite()
        || zoom <= 0.0
        || !tolerance.is_finite()
        || tolerance <= 0.0
    {
        return path;
    }
    let Ok(obstacles) = expanded(&input, config, budget) else {
        return path;
    };
    let Ok(()) = budget.spend(path.segments().len()) else {
        return path;
    };
    let mut points = Vec::new();
    for segment in path.segments() {
        let PathSegment::Line([a, b]) = segment else {
            return path;
        };
        if !a.is_finite() || !b.is_finite() || (a.x != b.x && a.y != b.y) {
            return path;
        }
        if points.is_empty() {
            points.push(*a);
        }
        if points.last() != Some(a) {
            return path;
        }
        if a != b {
            points.push(*b);
        }
    }
    if points.len() < 4 {
        return path;
    }
    let Ok(source_escape) = escape(input.source, input.nodes, &obstacles, config, budget) else {
        return path;
    };
    let Ok(target_escape) = escape(input.target, input.nodes, &obstacles, config, budget) else {
        return path;
    };
    // Coalesce lattice steps only inside the two protected escape endpoints.
    let mut interior: Vec<Pos2> = Vec::new();
    for &point in &points[1..points.len() - 1] {
        if interior.len() >= 2 {
            let a = interior[interior.len() - 2];
            let b = interior[interior.len() - 1];
            if (a.x == b.x && b.x == point.x && (b.y - a.y).signum() == (point.y - b.y).signum())
                || (a.y == b.y
                    && b.y == point.y
                    && (b.x - a.x).signum() == (point.x - b.x).signum())
            {
                interior.pop();
            }
        }
        interior.push(point);
    }
    let mut compact = vec![points[0]];
    compact.extend(interior);
    compact.push(*points.last().unwrap());
    let mut corners = vec![None; compact.len()];
    let mut changed = false;
    // Endpoint fillets may trim reserved room, never the mandatory escape.
    // All curves, including these transitions, have no own-node exemption.
    for i in 1..compact.len() - 1 {
        let a = compact[i - 1];
        let b = compact[i];
        let c = compact[i + 1];
        let direction = |a: Pos2, b: Pos2| {
            Vec2::new(
                if b.x == a.x {
                    0.0
                } else {
                    (b.x as f64 - a.x as f64).signum() as f32
                },
                if b.y == a.y {
                    0.0
                } else {
                    (b.y as f64 - a.y as f64).signum() as f32
                },
            )
        };
        let incoming = direction(a, b);
        let outgoing = direction(b, c);
        if incoming.dot(outgoing) != 0.0 {
            continue;
        }
        let distance =
            |a: Pos2, b: Pos2| (a.x as f64 - b.x as f64).abs() + (a.y as f64 - b.y as f64).abs();
        let mut radius = (config.corner_radius as f64)
            .min(distance(a, b) / 3.0)
            .min(distance(b, c) / 3.0) as f32;
        for _ in 0..6 {
            let Ok(()) = budget.spend(1) else { return path };
            let start = b - incoming * radius;
            let end = b + outgoing * radius;
            let handles = radius * 0.5522848;
            let curve = [
                start,
                start + incoming * handles,
                end - outgoing * handles,
                end,
            ];
            if start != b
                && end != b
                && curve[0] != curve[1]
                && curve[2] != curve[3]
                && Rect::from_two_pos(a, b).contains(start)
                && Rect::from_two_pos(b, c).contains(end)
                && (i != 1 || Rect::from_two_pos(a, start).contains(source_escape))
                && (i != compact.len() - 2 || Rect::from_two_pos(end, c).contains(target_escape))
            {
                match cubic_clear(curve, &obstacles, budget) {
                    Ok(true) => {
                        corners[i] = Some(curve);
                        changed = true;
                        break;
                    }
                    Ok(false) => {}
                    Err(_) => return path,
                }
            }
            radius *= 0.5;
        }
    }
    if !changed {
        return path;
    }
    let mut segments = Vec::new();
    let mut current = compact[0];
    for (i, &point) in compact.iter().enumerate().skip(1) {
        if let Some(curve) = corners[i] {
            if current != curve[0] {
                segments.push(PathSegment::Line([current, curve[0]]));
            }
            segments.push(PathSegment::Cubic(curve));
            current = curve[3];
        } else {
            if current != point {
                segments.push(PathSegment::Line([current, point]));
            }
            current = point;
        }
    }
    WirePath::new(segments, tolerance)
}
