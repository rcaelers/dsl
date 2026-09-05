//! Common-X cubic interpolation of checked monotonic bundles.

use egui::{Pos2, Rect};

use super::bundle::{BundleMember, endpoint_fan_width};
use super::contract::{RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::corridor::{cubic_clear, expanded};
use super::geometry::{PathSegment, WirePath};
use super::variable_spacing::widen_spacing;

/// Quality is atomic for the group: never mix independently smoothed lanes with
/// its checked lanes. The input paths must be the checked result for these members.
pub(crate) fn smooth_bundle(
    nodes: &[Rect],
    members: &[BundleMember],
    paths: Vec<WirePath>,
    config: &RouteConfig,
    zoom: f32,
    budget: &mut WorkBudget,
) -> Vec<WirePath> {
    match candidate(nodes, members, &paths, config, zoom, budget) {
        Ok(Some(smoothed)) => smoothed,
        Ok(None) | Err(_) => paths,
    }
}

fn candidate(
    nodes: &[Rect],
    members: &[BundleMember],
    paths: &[WirePath],
    config: &RouteConfig,
    zoom: f32,
    budget: &mut WorkBudget,
) -> Result<Option<Vec<WirePath>>, RouteFailure> {
    let tolerance = 0.5 / zoom;
    if paths.len() < 2
        || paths.len() != members.len()
        || !tolerance.is_finite()
        || tolerance <= 0.0
        || !config.corner_radius.is_finite()
        || config.corner_radius <= 0.0
    {
        return Ok(None);
    }
    let obstacles = expanded(
        &RouteInput {
            nodes,
            source: members[0].source,
            target: members[0].target,
        },
        config,
        budget,
    )?;
    let mut lines = Vec::with_capacity(paths.len());
    for (path, member) in paths.iter().zip(members) {
        budget.spend(path.segments().len())?;
        let mut lane: Vec<[Pos2; 2]> = Vec::new();
        for segment in path.segments() {
            let PathSegment::Line(p) = segment else {
                return Ok(None);
            };
            if p.iter().any(|p| !p.is_finite())
                || p[0].x > p[1].x
                || (p[0].x != p[1].x && p[0].y != p[1].y)
                || lane.last().is_some_and(|previous| previous[1] != p[0])
            {
                return Ok(None);
            }
            lane.push(*p);
        }
        if lane.len() < 3
            || lane[0][0] != member.source.position
            || lane.last().unwrap()[1] != member.target.position
        {
            return Ok(None);
        }
        lines.push(lane);
    }
    let entry = lines[0][0][1].x;
    let exit = lines[0].last().unwrap()[0].x;
    if entry >= exit
        || lines.iter().any(|p| {
            p[0][1].x != entry
                || p.last().unwrap()[0].x != exit
                || p[0][0].y != p[0][1].y
                || p.last().unwrap()[0].y != p.last().unwrap()[1].y
        })
    {
        return Ok(None);
    }
    let fan_width = endpoint_fan_width(members.len(), config.lane_spacing);
    let interior = [entry + fan_width, exit - fan_width];
    if interior[0] >= interior[1] {
        return Ok(None);
    }
    let mut radius = config.corner_radius;
    for _ in 0..12 {
        let mut windows = Vec::new();
        for lane in &lines {
            for p in &lane[1..lane.len() - 1] {
                budget.spend(1)?;
                if p[0].x == p[1].x && p[0].y != p[1].y {
                    windows.push([(p[0].x - radius).max(entry), (p[0].x + radius).min(exit)]);
                }
            }
        }
        if windows.is_empty() {
            return Ok(None);
        }
        windows.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        let mut merged: Vec<[f32; 2]> = Vec::new();
        for window in windows {
            if let Some(previous) = merged.last_mut()
                && window[0] <= previous[1]
            {
                previous[1] = previous[1].max(window[1]);
            } else {
                merged.push(window);
            }
        }
        let mut knots = vec![entry, exit, interior[0], interior[1]];
        knots.extend(merged.into_iter().flatten());
        // Capacity can change on a straight run as well as at a turn. Add
        // shared knots on both sides of each expanded obstacle boundary.
        if config.preferred_lane_spacing > config.lane_spacing {
            for obstacle in &obstacles {
                budget.spend(4)?;
                for boundary in [obstacle.min.x, obstacle.max.x] {
                    for x in [
                        boundary - config.corner_radius,
                        boundary + config.corner_radius,
                    ] {
                        if x.is_finite() && x > interior[0] && x < interior[1] {
                            knots.push(x);
                        }
                    }
                }
            }
        }
        knots.sort_by(f32::total_cmp);
        knots.dedup_by(|a, b| *a == *b);
        if config.preferred_lane_spacing > config.lane_spacing {
            budget.spend(knots.len())?;
            let midpoints: Vec<_> = knots
                .windows(2)
                .filter_map(|p| {
                    let x = (p[0] as f64 + (p[1] as f64 - p[0] as f64) / 2.0) as f32;
                    (p[0] >= interior[0] && p[1] <= interior[1] && x > p[0] && x < p[1])
                        .then_some(x)
                })
                .collect();
            knots.extend(midpoints);
            knots.sort_by(f32::total_cmp);
        }
        let xs: Vec<_> = knots
            .windows(2)
            .map(|p| {
                [
                    p[0],
                    (p[0] as f64 + (p[1] as f64 - p[0] as f64) / 3.0) as f32,
                    (p[1] as f64 - (p[1] as f64 - p[0] as f64) / 3.0) as f32,
                    p[1],
                ]
            })
            .collect();
        if xs.iter().any(|p| {
            !p.iter().all(|x| x.is_finite()) || p[0] >= p[1] || p[1] > p[2] || p[2] >= p[3]
        }) {
            radius *= 0.5;
            continue;
        }
        let mut curves = Vec::new();
        for lane in &lines {
            let mut ys = Vec::new();
            for &x in &knots {
                // The right-hand value at an exact vertical event is deterministic.
                // Smoothing may shortcut an excursion, but only collision proof can accept it.
                let mut y = None;
                for p in &lane[1..lane.len() - 1] {
                    budget.spend(1)?;
                    if p[0].x <= x && x <= p[1].x {
                        y = Some(p[1].y);
                    }
                }
                let Some(y) = y else { return Ok(None) };
                ys.push(y);
            }
            if ys.first() != Some(&lane[0][1].y) || ys.last() != Some(&lane.last().unwrap()[0].y) {
                // An exact vertical event at an escape cannot be replaced by a
                // disconnected curve. Keep the checked group in this case.
                return Ok(None);
            }
            curves.push(
                xs.iter()
                    .zip(ys.windows(2))
                    .map(|(x, y)| {
                        [
                            Pos2::new(x[0], y[0]),
                            Pos2::new(x[1], y[0]),
                            Pos2::new(x[2], y[1]),
                            Pos2::new(x[3], y[1]),
                        ]
                    })
                    .collect::<Vec<_>>(),
            );
        }
        if !ordered(&curves, members, interior, config.lane_spacing, budget)? {
            radius *= 0.5;
            continue;
        }
        let mut clear = true;
        for lane in &curves {
            for curve in lane {
                if !cubic_clear(*curve, &obstacles, budget)? {
                    clear = false;
                    break;
                }
            }
            if !clear {
                break;
            }
        }
        if clear {
            widen_spacing(
                &mut curves,
                &obstacles,
                interior,
                config.preferred_lane_spacing,
                budget,
            );
            return Ok(Some(
                curves
                    .into_iter()
                    .zip(lines)
                    .map(|(lane, original)| {
                        let mut segments = vec![PathSegment::Line(original[0])];
                        segments.extend(lane.into_iter().map(PathSegment::Cubic));
                        segments.push(PathSegment::Line(*original.last().unwrap()));
                        WirePath::new(segments, tolerance)
                    })
                    .collect(),
            ));
        }
        radius *= 0.5;
    }
    Ok(None)
}

fn ordered(
    curves: &[Vec<[Pos2; 4]>],
    members: &[BundleMember],
    interior: [f32; 2],
    spacing: f32,
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    for (i, pair) in curves.windows(2).enumerate() {
        let same = |section: usize| (0..4).all(|k| pair[0][section][k].y == pair[1][section][k].y);
        budget.spend(pair[0].len().saturating_mul(4))?;
        let first_difference = (0..pair[0].len())
            .find(|&s| !same(s))
            .unwrap_or(pair[0].len());
        let last_difference = (0..pair[0].len()).rfind(|&s| !same(s));
        let shared_source = members[i].source_socket == members[i + 1].source_socket
            && members[i].source.position == members[i + 1].source.position;
        let shared_target = members[i].target_socket == members[i + 1].target_socket
            && members[i].target.position == members[i + 1].target.position;
        for (s, (a, b)) in pair[0].iter().zip(&pair[1]).enumerate() {
            budget.spend(4)?;
            let minimum = if a[0].x >= interior[0] && a[3].x <= interior[1] {
                spacing as f64
            } else {
                0.0
            };
            for k in 0..4 {
                // Common X controls give a common strictly increasing X(t). Ordered
                // Y coefficients and nonnegative Bernstein weights prove whole-curve order.
                if a[k].x != b[k].x || b[k].y as f64 - (a[k].y as f64) < minimum {
                    return Ok(false);
                }
            }
            for (k, knot) in [(0, s), (3, s + 1)] {
                if a[k].y == b[k].y {
                    let prefix = shared_source && knot <= first_difference;
                    let suffix = shared_target && last_difference.is_none_or(|last| knot > last);
                    if !prefix && !suffix {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}
