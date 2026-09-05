//! Checked lane bundles with rectilinear, ordered endpoint fan-outs.
//! Failure means this bounded family of corridors did not fit, not that none exists.

use egui::{Pos2, Rect};

use super::bundle_corridor::route_lanes;
use super::contract::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::corridor::{ObstacleSubset, escape, expanded};
use super::geometry::{PathSegment, WirePath};

pub(crate) struct BundleMember {
    pub(crate) source: PortGeometry,
    pub(crate) target: PortGeometry,
    /// Explicit endpoint identity within the node pair; coincident ports are not shared ports.
    pub(crate) source_socket: usize,
    pub(crate) target_socket: usize,
}

pub(crate) fn endpoint_fan_width(count: usize, spacing: f32) -> f32 {
    (count + 1) as f32 * spacing
}

pub(crate) fn route_bundle(
    nodes: &[Rect],
    members: &[BundleMember],
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> Result<Vec<WirePath>, RouteFailure> {
    let first = members.first().ok_or(RouteFailure::InvalidGeometry)?;
    if members.len() < 2
        || members.iter().any(|member| {
            member.source.obstacle != first.source.obstacle
                || member.target.obstacle != first.target.obstacle
                || member.source.side != PortSide::Right
                || member.target.side != PortSide::Left
        })
    {
        return Err(RouteFailure::InvalidGeometry);
    }
    budget.spend(members.len())?;
    let obstacles = expanded(
        &RouteInput {
            nodes,
            source: first.source,
            target: first.target,
        },
        config,
        budget,
    )?;
    let mut sources = Vec::with_capacity(members.len());
    let mut targets = Vec::with_capacity(members.len());
    for member in members {
        sources.push(escape(member.source, nodes, &obstacles, config, budget)?);
        targets.push(escape(member.target, nodes, &obstacles, config, budget)?);
    }
    if sources.windows(2).any(|p| p[0].y > p[1].y) || targets.windows(2).any(|p| p[0].y > p[1].y) {
        return Err(RouteFailure::NoCorridor);
    }
    let source_x = sources
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let target_x = targets.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let height = (members.len() - 1) as f32 * config.lane_spacing;
    let fan_width = endpoint_fan_width(members.len(), config.lane_spacing);
    let left = source_x + fan_width;
    let right = target_x - fan_width;
    if !height.is_finite() || !left.is_finite() || !right.is_finite() || left >= right {
        return Err(RouteFailure::NoCorridor);
    }
    // Use f64 while averaging finite endpoints to avoid overflow before conversion.
    let ideal = ((sources[0].y as f64
        + sources.last().unwrap().y as f64
        + targets[0].y as f64
        + targets.last().unwrap().y as f64)
        * 0.25
        - height as f64 * 0.5) as f32;
    budget.spend(obstacles.len().saturating_mul(2).saturating_add(1))?;
    let mut bands = vec![ideal];
    for obstacle in &obstacles {
        bands.push((obstacle.min.y - height - config.safety).next_down());
        bands.push((obstacle.max.y + config.safety).next_up());
    }
    bands.retain(|y| y.is_finite() && (y + height).is_finite());
    bands.sort_by(|a, b| {
        ((*a as f64 - ideal as f64).abs())
            .total_cmp(&(*b as f64 - ideal as f64).abs())
            .then(a.total_cmp(b))
    });
    bands.dedup();
    let geometry = BundleGeometry {
        members,
        sources: &sources,
        targets: &targets,
        obstacles: &obstacles,
        source_x,
        target_x,
    };
    for &top in &bands {
        budget.spend(members.len())?;
        let lanes: Vec<_> = (0..members.len())
            .map(|i| top + i as f32 * config.lane_spacing)
            .collect();
        // Floating point rounding at very large coordinates must not collapse spacing.
        if lanes
            .windows(2)
            .any(|p| p[1] as f64 - (p[0] as f64) < config.lane_spacing as f64)
        {
            continue;
        }
        let bottom = *lanes.last().unwrap();
        let envelopes = [
            Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
            Rect::from_min_max(
                Pos2::new(source_x, top.min(sources[0].y)),
                Pos2::new(left, bottom.max(sources.last().unwrap().y)),
            ),
            Rect::from_min_max(
                Pos2::new(right, top.min(targets[0].y)),
                Pos2::new(target_x, bottom.max(targets.last().unwrap().y)),
            ),
        ];
        // Test the entire band AND its connecting openings, not just sampled lane centers.
        let mut fits = true;
        for envelope in envelopes {
            for obstacle in &obstacles {
                budget.spend(1)?;
                if envelope.intersects(*obstacle) {
                    fits = false;
                    break;
                }
            }
            if !fits {
                break;
            }
        }
        if !fits {
            continue;
        }
        let interiors = lanes
            .iter()
            .map(|&y| vec![Pos2::new(left, y), Pos2::new(right, y)])
            .collect();
        match geometry.checked_paths(interiors, config, budget) {
            Err(RouteFailure::NoCorridor) => {}
            result => return result,
        }
    }
    let source_top =
        ((sources[0].y as f64 + sources.last().unwrap().y as f64 - height as f64) * 0.5) as f32;
    let target_top =
        ((targets[0].y as f64 + targets.last().unwrap().y as f64 - height as f64) * 0.5) as f32;
    bands.extend([source_top, target_top]);
    bands.retain(|y| y.is_finite() && (y + height).is_finite());
    let mut target_bands = bands.clone();
    for (bands, ideal) in [(&mut bands, source_top), (&mut target_bands, target_top)] {
        bands.sort_by(|a, b| {
            ((*a as f64 - ideal as f64).abs())
                .total_cmp(&(*b as f64 - ideal as f64).abs())
                .then(a.total_cmp(b))
        });
        bands.dedup();
    }
    for source_top in bands {
        if !geometry.fan_clear(true, source_top, left, height, budget)? {
            continue;
        }
        for &target_top in &target_bands {
            if !geometry.fan_clear(false, target_top, right, height, budget)? {
                continue;
            }
            let result = route_lanes(
                Pos2::new(left, source_top),
                Pos2::new(right, target_top),
                &obstacles,
                members.len(),
                config,
                budget,
            )
            .and_then(|interiors| geometry.checked_paths(interiors, config, budget));
            match result {
                Err(RouteFailure::NoCorridor) => {}
                result => return result,
            }
        }
    }
    Err(RouteFailure::NoCorridor)
}

struct BundleGeometry<'a> {
    members: &'a [BundleMember],
    sources: &'a [Pos2],
    targets: &'a [Pos2],
    obstacles: &'a [Rect],
    source_x: f32,
    target_x: f32,
}

impl BundleGeometry<'_> {
    fn fan_clear(
        &self,
        source: bool,
        top: f32,
        edge: f32,
        height: f32,
        budget: &mut WorkBudget,
    ) -> Result<bool, RouteFailure> {
        let (ports, x) = if source {
            (self.sources, self.source_x)
        } else {
            (self.targets, self.target_x)
        };
        let envelope = Rect::from_min_max(
            Pos2::new(x.min(edge), top.min(ports[0].y)),
            Pos2::new(x.max(edge), (top + height).max(ports.last().unwrap().y)),
        );
        for obstacle in self.obstacles {
            budget.spend(1)?;
            if envelope.intersects(*obstacle) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn checked_paths(
        &self,
        interiors: Vec<Vec<Pos2>>,
        config: &RouteConfig,
        budget: &mut WorkBudget,
    ) -> Result<Vec<WirePath>, RouteFailure> {
        let source_lanes: Vec<_> = interiors.iter().map(|p| p[0].y).collect();
        let target_lanes: Vec<_> = interiors.iter().map(|p| p.last().unwrap().y).collect();
        let source_turns = fan_columns(
            self.sources,
            &source_lanes,
            self.source_x,
            config.lane_spacing,
        );
        let target_turns = fan_columns(
            self.targets,
            &target_lanes,
            self.target_x,
            -config.lane_spacing,
        );
        let mut candidate_points = Vec::with_capacity(self.members.len());
        let mut bounds = Rect::NOTHING;
        for (i, (member, interior)) in self.members.iter().zip(interiors).enumerate() {
            let mut points = vec![
                member.source.position,
                self.sources[i],
                Pos2::new(source_turns[i], self.sources[i].y),
                Pos2::new(source_turns[i], source_lanes[i]),
            ];
            points.extend(interior);
            points.extend([
                Pos2::new(target_turns[i], target_lanes[i]),
                Pos2::new(target_turns[i], self.targets[i].y),
                self.targets[i],
                member.target.position,
            ]);
            budget.spend(points.len())?;
            for &point in &points {
                if !point.is_finite() {
                    return Err(RouteFailure::InvalidGeometry);
                }
                bounds.extend_with(point);
            }
            candidate_points.push(points);
        }
        // All lanes and their endpoint escapes lie in this closed envelope.
        // Scan the complete obstacle set once, retaining original exemption IDs;
        // exact segment checks still decide every potentially intersecting body.
        let obstacles = ObstacleSubset::new(self.obstacles, bounds, budget)?;
        let mut paths = Vec::with_capacity(self.members.len());
        for (member, points) in self.members.iter().zip(candidate_points) {
            let mut segments = Vec::new();
            for (index, pair) in points.windows(2).enumerate() {
                let exempt = if index == 0 {
                    Some(member.source.obstacle)
                } else if index == points.len() - 2 {
                    Some(member.target.obstacle)
                } else {
                    None
                };
                if pair[0].x > pair[1].x || !obstacles.clear([pair[0], pair[1]], exempt, budget)? {
                    return Err(RouteFailure::NoCorridor);
                }
                if pair[0] != pair[1] {
                    segments.push(PathSegment::Line([pair[0], pair[1]]));
                }
            }
            paths.push(WirePath::new(segments, 0.5));
        }
        if separated(&paths, self.members, &source_turns, &target_turns, budget)? {
            Ok(paths)
        } else {
            Err(RouteFailure::NoCorridor)
        }
    }
}

/// Moving upward: top lanes turn first. Moving downward: bottom lanes turn first.
/// Reversing X constructs the target fan by the same rule, then paths traverse it backward.
fn fan_columns(ports: &[Pos2], lanes: &[f32], start: f32, step: f32) -> Vec<f32> {
    let mut order: Vec<_> = (0..ports.len()).collect();
    order.sort_by_key(|&i| {
        if lanes[i] < ports[i].y {
            (0, i)
        } else {
            (1, ports.len() - i)
        }
    });
    let mut columns = vec![0.0; ports.len()];
    for (rank, lane) in order.into_iter().enumerate() {
        columns[lane] = start + (rank + 1) as f32 * step;
    }
    columns
}

/// Analytic segment-pair checks also reject folded or coincident fan-outs. Only an
/// explicitly shared endpoint's horizontal prefix/suffix may overlap, including its fork.
fn separated(
    paths: &[WirePath],
    members: &[BundleMember],
    source_turns: &[f32],
    target_turns: &[f32],
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    for (i, a) in paths.iter().enumerate() {
        for (j, b) in paths.iter().enumerate().skip(i + 1) {
            for a in a.segments() {
                for b in b.segments() {
                    budget.spend(1)?;
                    let (PathSegment::Line(a), PathSegment::Line(b)) = (a, b) else {
                        return Ok(false);
                    };
                    let intersection =
                        Rect::from_two_pos(a[0], a[1]).intersect(Rect::from_two_pos(b[0], b[1]));
                    if intersection.is_negative() {
                        continue;
                    }
                    let source = members[i].source.position;
                    let target = members[i].target.position;
                    let shared_source = members[i].source_socket == members[j].source_socket
                        && source == members[j].source.position
                        && intersection.min.y == source.y
                        && intersection.max.y == source.y
                        && intersection.min.x >= source.x
                        && intersection.max.x <= source_turns[i].min(source_turns[j]);
                    let shared_target = members[i].target_socket == members[j].target_socket
                        && target == members[j].target.position
                        && intersection.min.y == target.y
                        && intersection.max.y == target.y
                        && intersection.min.x >= target_turns[i].max(target_turns[j])
                        && intersection.max.x <= target.x;
                    if !shared_source && !shared_target {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}
