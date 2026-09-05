//! Optional endpoint-transition room without spending the checked-search budget.

use super::contract::{RouteConfig, RouteInput, WorkBudget};
use super::geometry::{PathSegment, WirePath};
use super::individual::route_with_budget;
use super::smoothing::smooth_route;

pub(crate) fn improve_route(
    input: RouteInput<'_>,
    path: WirePath,
    config: &RouteConfig,
    zoom: f32,
    budget: &mut WorkBudget,
) -> WirePath {
    if !config.corner_radius.is_finite() || config.corner_radius <= 0.0 {
        return path;
    }
    let turns = |pair: &[PathSegment]| {
        let [PathSegment::Line([a, b]), PathSegment::Line([c, d])] = pair else {
            return false;
        };
        b == c && a != b && c != d && (a.x == b.x) != (c.x == d.x)
    };
    let segments = path.segments();
    if segments.len() < 2 || (!turns(&segments[..2]) && !turns(&segments[segments.len() - 2..])) {
        return smooth_route(input, path, config, zoom, budget);
    }
    // Two radii leave a straight prefix beyond the mandatory escape after a
    // fillet trims the extended escape. The checked solver validates the new
    // escape against other nodes and reselects the corridor around its endpoint.
    let reserved = RouteConfig {
        escape: config.escape.max(config.clearance_x + config.safety) + 2.0 * config.corner_radius,
        ..*config
    };
    let candidate = route_with_budget(
        RouteInput {
            nodes: input.nodes,
            source: input.source,
            target: input.target,
        },
        &reserved,
        budget,
    )
    .unwrap_or(path);
    smooth_route(input, candidate, config, zoom, budget)
}
