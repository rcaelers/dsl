use egui::{Pos2, Rect, Vec2};

use super::super::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};

pub(crate) fn expanded(
    input: &RouteInput<'_>,
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> Result<Vec<Rect>, RouteFailure> {
    if ![config.clearance_x, config.clearance_y, config.escape]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0)
        || !config.safety.is_finite()
        || config.safety <= 0.0
        || !config.lane_spacing.is_finite()
        || config.lane_spacing <= 0.0
        || !config.bend_cost.is_finite()
        || config.bend_cost < 0.0
        || !config.vertical_weight.is_finite()
        || config.vertical_weight < 1.0
    {
        return Err(RouteFailure::InvalidGeometry);
    }
    budget.spend(input.nodes.len())?;
    input
        .nodes
        .iter()
        .map(|rect| expand_obstacle(*rect, config))
        .collect()
}

pub(crate) fn expand_obstacle(rect: Rect, config: &RouteConfig) -> Result<Rect, RouteFailure> {
    if !valid_rect(rect) {
        return Err(RouteFailure::InvalidGeometry);
    }
    let expanded = rect.expand2(Vec2::new(config.clearance_x, config.clearance_y));
    if !valid_rect(expanded) {
        return Err(RouteFailure::InvalidGeometry);
    }
    Ok(expanded)
}

fn valid_rect(rect: Rect) -> bool {
    rect.min.is_finite()
        && rect.max.is_finite()
        && rect.min.x < rect.max.x
        && rect.min.y < rect.max.y
}

pub(crate) fn escape(
    port: PortGeometry,
    nodes: &[Rect],
    obstacles: &[Rect],
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> Result<Pos2, RouteFailure> {
    let body = nodes
        .get(port.obstacle)
        .ok_or(RouteFailure::InvalidGeometry)?;
    if !port.position.is_finite() || port.position.y < body.min.y || port.position.y > body.max.y {
        return Err(RouteFailure::InvalidGeometry);
    }
    let x = match port.side {
        PortSide::Left if port.position.x == body.min.x => (obstacles[port.obstacle].min.x
            - config.safety)
            .min(port.position.x - config.escape)
            .next_down(),
        PortSide::Right if port.position.x == body.max.x => (obstacles[port.obstacle].max.x
            + config.safety)
            .max(port.position.x + config.escape)
            .next_up(),
        _ => return Err(RouteFailure::InvalidGeometry),
    };
    let end = Pos2::new(x, port.position.y);
    if !end.is_finite() {
        return Err(RouteFailure::InvalidGeometry);
    }
    // Only this outward segment is exempt from its own expanded node. Its start is
    // on the declared body boundary and its direction proves it avoids body interior.
    if !clear([port.position, end], obstacles, Some(port.obstacle), budget)? {
        return Err(RouteFailure::BlockedEscape);
    }
    Ok(end)
}

/// Exact closed-rectangle collision for rectilinear segments, including corner contact.
pub(crate) fn clear(
    segment: [Pos2; 2],
    obstacles: &[Rect],
    exempt: Option<usize>,
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    if !segment.iter().all(|point| point.is_finite())
        || (segment[0].x != segment[1].x && segment[0].y != segment[1].y)
    {
        return Err(RouteFailure::InvalidGeometry);
    }
    let bounds = Rect::from_two_pos(segment[0], segment[1]);
    for (index, obstacle) in obstacles.iter().enumerate() {
        budget.spend(1)?;
        if Some(index) != exempt && bounds.intersects(*obstacle) {
            return Ok(false);
        }
    }
    Ok(true)
}
