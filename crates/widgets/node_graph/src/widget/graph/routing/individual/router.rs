use super::super::{PathSegment, WirePath};
use super::contract::{PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::obstacle::{clear, escape, expanded};
use super::search::Channels;

pub(crate) fn route(input: RouteInput<'_>, config: &RouteConfig) -> Result<WirePath, RouteFailure> {
    let mut budget = WorkBudget::new(config.max_work);
    let obstacles = expanded(&input, config, &mut budget)?;
    let start = escape(input.source, input.nodes, &obstacles, config, &mut budget)?;
    let end = escape(input.target, input.nodes, &obstacles, config, &mut budget)?;
    let channels = Channels::new(start, end, &obstacles, config, &mut budget)?;
    let monotonic = input.source.side == PortSide::Right
        && input.target.side == PortSide::Left
        && start.x < end.x;
    let interior = if monotonic {
        match channels.find(start, end, &obstacles, config, true, &mut budget) {
            Err(RouteFailure::NoCorridor) => {
                channels.find(start, end, &obstacles, config, false, &mut budget)
            }
            result => result,
        }
    } else {
        channels.find(start, end, &obstacles, config, false, &mut budget)
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
        if !clear([pair[0], pair[1]], &obstacles, exempt, &mut budget)? {
            return Err(RouteFailure::NoCorridor);
        }
        segments.push(PathSegment::Line([pair[0], pair[1]]));
    }
    Ok(WirePath::new(segments, 0.5))
}
