//! Separate independent signals without turning their crossings into obstacles.

use egui::{Pos2, Rect};

use super::contract::{PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::corridor::{Channels, clear, escape, expanded, parallel_overlap};
use super::geometry::{PathSegment, WirePath};
use super::smoothing::smooth_route;

fn straight(segment: &PathSegment) -> Option<[Pos2; 2]> {
    match segment {
        PathSegment::Line(p) => Some(*p),
        // Straight cubics are common in smoothed ordered bundles. Their control
        // hull contains the entire run, including collinear reversals.
        PathSegment::Cubic(p)
            if p.iter().all(|v| v.x == p[0].x) || p.iter().all(|v| v.y == p[0].y) =>
        {
            let mut bounds = Rect::NOTHING;
            for &point in p {
                bounds.extend_with(point);
            }
            Some([bounds.min, bounds.max])
        }
        _ => None,
    }
}

pub(crate) fn shares_run(
    a: &WirePath,
    b: &WirePath,
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    budget.spend(1)?;
    if !a.bounds().intersects(b.bounds()) {
        return Ok(false);
    }
    for a in a.segments() {
        for b in b.segments() {
            budget.spend(1)?;
            if let (Some(a), Some(b)) = (straight(a), straight(b)) {
                if parallel_overlap(a, b, 0.0) {
                    return Ok(true);
                }
            } else if let (PathSegment::Cubic(a), PathSegment::Cubic(b)) = (a, b)
                && (a == b || a.iter().eq(b.iter().rev()))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// `others` contains only different source sockets. The bool protects members
/// of the same compatible bundle from new crossings as well as shared runs.
pub(crate) fn separate_route(
    input: RouteInput<'_>,
    others: &[(&WirePath, bool)],
    config: &RouteConfig,
    zoom: f32,
    budget: &mut WorkBudget,
) -> Result<WirePath, RouteFailure> {
    if !zoom.is_finite() || zoom <= 0.0 || !(0.5 / zoom).is_finite() {
        return Err(RouteFailure::InvalidGeometry);
    }
    let mut obstacles = expanded(&input, config, budget)?;
    let mut reserved = Vec::new();
    for &(path, protect_crossings) in others {
        for segment in path.segments() {
            budget.spend(1)?;
            if let Some(line) = straight(segment) {
                reserved.push(line);
            }
            // Nonlinear arcs cannot share an interval with a rectilinear retry.
            // Bundle peers additionally reserve their complete control hulls to
            // preserve lane order conservatively, without sampling the curves.
            if protect_crossings {
                let mut bounds = Rect::NOTHING;
                match segment {
                    PathSegment::Line(p) => {
                        for &v in p {
                            bounds.extend_with(v);
                        }
                    }
                    PathSegment::Cubic(p) => {
                        for &v in p {
                            bounds.extend_with(v);
                        }
                    }
                }
                obstacles.push(bounds.expand(config.lane_spacing.max(config.safety)));
            }
        }
    }
    let start = escape(input.source, input.nodes, &obstacles, config, budget)?;
    let end = escape(input.target, input.nodes, &obstacles, config, budget)?;
    let channels = Channels::avoiding_runs(start, end, &obstacles, &reserved, config, budget)?
        .with_endpoint_sides(input.source.side, input.target.side);
    let monotonic = input.source.side == PortSide::Right
        && input.target.side == PortSide::Left
        && start.x < end.x;
    let interior = if monotonic {
        match channels.find(start, end, &obstacles, config, true, budget) {
            Err(RouteFailure::NoCorridor) => {
                channels.find(start, end, &obstacles, config, false, budget)
            }
            result => result,
        }
    } else {
        channels.find(start, end, &obstacles, config, false, budget)
    }?;
    let mut points = vec![input.source.position];
    points.extend(interior);
    points.push(input.target.position);
    let mut segments = Vec::new();
    for (index, pair) in points.windows(2).enumerate() {
        let line = [pair[0], pair[1]];
        let exempt = if index == 0 {
            Some(input.source.obstacle)
        } else if index == points.len() - 2 {
            Some(input.target.obstacle)
        } else {
            None
        };
        if !clear(line, &obstacles, exempt, budget)? || !channels.run_clear(line, budget)? {
            return Err(RouteFailure::NoCorridor);
        }
        segments.push(PathSegment::Line(line));
    }
    let checked = WirePath::new(segments, 0.5 / zoom);
    // Smoothing is optional, and must not reintroduce a shared curve or cross a
    // protected lane. Its node proof alone does not establish wire separation.
    if others.iter().any(|(_, protected)| *protected) {
        return Ok(checked);
    }
    let smoothed = smooth_route(input, checked.clone(), config, zoom, budget);
    for &(other, _) in others {
        if shares_run(&smoothed, other, budget) != Ok(false) {
            return Ok(checked);
        }
    }
    Ok(smoothed)
}
