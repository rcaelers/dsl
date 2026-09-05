use super::super::corridor::{Channels, clear, escape, expanded};
use super::super::{
    PathSegment, PortSide, RouteConfig, RouteFailure, RouteInput, WirePath, WorkBudget,
};

#[cfg(test)]
pub(crate) fn route(input: RouteInput<'_>, config: &RouteConfig) -> Result<WirePath, RouteFailure> {
    let mut budget = WorkBudget::new(config.max_work);
    route_with_budget(input, config, &mut budget)
}

pub(crate) fn route_with_budget(
    input: RouteInput<'_>,
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> Result<WirePath, RouteFailure> {
    let obstacles = expanded(&input, config, budget)?;
    let start = escape(input.source, input.nodes, &obstacles, config, budget)?;
    let end = escape(input.target, input.nodes, &obstacles, config, budget)?;
    let channels = Channels::new(start, end, &obstacles, config, budget)?;
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
    let mut points = Vec::with_capacity(interior.len() + 2);
    points.push(input.source.position);
    points.extend(interior);
    points.push(input.target.position);
    let mut segments = Vec::with_capacity(points.len() - 1);
    for (index, pair) in points.windows(2).enumerate() {
        let exempt = if index == 0 {
            Some(input.source.obstacle)
        } else if index == points.len() - 2 {
            Some(input.target.obstacle)
        } else {
            None
        };
        // Check the exact final geometry, not the hit-testing approximation. Only the
        // first/last segments receive their own-node exception, never the interior.
        if !clear([pair[0], pair[1]], &obstacles, exempt, budget)? {
            return Err(RouteFailure::NoCorridor);
        }
        segments.push(PathSegment::Line([pair[0], pair[1]]));
    }
    Ok(WirePath::new(segments, 0.5))
}
