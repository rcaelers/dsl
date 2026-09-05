//! Budget-isolated corridor preferences layered over the checked minimum route.

use egui::Rect;

use super::bundle::{BundleMember, route_bundle};
use super::contract::{RouteConfig, RouteFailure, WorkBudget};
use super::geometry::WirePath;
use super::ordered_smoothing::smooth_bundle;

pub(crate) fn route_quality_bundle(
    nodes: &[Rect],
    members: &[BundleMember],
    config: &RouteConfig,
    zoom: f32,
    route_budget: &mut WorkBudget,
    quality_budget: &mut WorkBudget,
) -> Result<Vec<WirePath>, RouteFailure> {
    // Establish safety first. Optional searches cannot consume another group's
    // checked-search allowance or turn a checked bundle into a routing failure.
    let baseline = route_bundle(nodes, members, config, route_budget)?;
    let (paths, selected) = prefer_corridor(nodes, members, baseline, config, quality_budget);
    // The reserved margin is construction room, not an increased collision
    // requirement. Keep the selected spacing (including its fan width) while
    // proving curves against the original clearance contract.
    let smoothing = RouteConfig {
        lane_spacing: selected.lane_spacing,
        ..*config
    };
    Ok(smooth_bundle(
        nodes,
        members,
        paths,
        &smoothing,
        zoom,
        quality_budget,
    ))
}

fn prefer_corridor(
    nodes: &[Rect],
    members: &[BundleMember],
    baseline: Vec<WirePath>,
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> (Vec<WirePath>, RouteConfig) {
    let preferred = config.preferred_lane_spacing;
    let margin = config.corner_radius;
    if !preferred.is_finite()
        || preferred < config.lane_spacing
        || !margin.is_finite()
        || margin <= 0.0
    {
        return (baseline, *config);
    }
    // Reserve a full corner radius before choosing a corridor. Relax spacing
    // first, then the reservation; the minimum unreserved route is already held.
    for (spacing, reserve) in [
        (preferred, margin),
        (config.lane_spacing, margin),
        (preferred, 0.0),
    ] {
        let candidate = RouteConfig {
            lane_spacing: spacing,
            clearance_x: config.clearance_x + reserve,
            clearance_y: config.clearance_y + reserve,
            ..*config
        };
        match route_bundle(nodes, members, &candidate, budget) {
            Ok(paths) => return (paths, candidate),
            Err(RouteFailure::WorkLimit) => break,
            Err(_) => {}
        }
    }
    (baseline, *config)
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Vec2};

    use super::super::contract::{PortGeometry, PortSide};
    use super::*;

    fn scene(gap: f32) -> (Vec<Rect>, Vec<BundleMember>) {
        let nodes = vec![
            Rect::from_min_size(Pos2::ZERO, Vec2::new(50.0, 100.0)),
            Rect::from_min_size(Pos2::new(50.0 + gap, 0.0), Vec2::new(50.0, 100.0)),
        ];
        let members = (0..2)
            .map(|i| BundleMember {
                source: PortGeometry {
                    obstacle: 0,
                    position: Pos2::new(50.0, 30.0 + 20.0 * i as f32),
                    side: PortSide::Right,
                },
                target: PortGeometry {
                    obstacle: 1,
                    position: Pos2::new(50.0 + gap, 30.0 + 20.0 * i as f32),
                    side: PortSide::Left,
                },
                source_socket: i,
                target_socket: i,
            })
            .collect();
        (nodes, members)
    }

    #[test]
    fn corridor_preferences_relax_spacing_then_reserved_clearance() {
        let config = RouteConfig::default();
        for (gap, spacing, clearance) in
            [(500.0, 12.0, 32.0), (115.0, 8.0, 32.0), (110.0, 8.0, 20.0)]
        {
            let (nodes, members) = scene(gap);
            let baseline = route_bundle(
                &nodes,
                &members,
                &config,
                &mut WorkBudget::new(config.max_work),
            )
            .unwrap();
            let (_, selected) = prefer_corridor(
                &nodes,
                &members,
                baseline,
                &config,
                &mut WorkBudget::new(config.max_smoothing_work),
            );
            assert_eq!(selected.lane_spacing, spacing, "gap {gap}");
            assert_eq!(selected.clearance_x, clearance, "gap {gap}");
        }
    }

    #[test]
    fn exhausted_quality_budget_preserves_checked_paths_and_search_allowance() {
        let config = RouteConfig::default();
        let (nodes, members) = scene(500.0);
        let mut baseline_budget = WorkBudget::new(5_000);
        let baseline = route_bundle(&nodes, &members, &config, &mut baseline_budget).unwrap();
        let mut route_budget = WorkBudget::new(5_000);
        let actual = route_quality_bundle(
            &nodes,
            &members,
            &config,
            1.0,
            &mut route_budget,
            &mut WorkBudget::new(0),
        )
        .unwrap();
        for (a, b) in actual.iter().zip(&baseline) {
            assert_eq!(format!("{:?}", a.segments()), format!("{:?}", b.segments()));
        }
        // Optional work must leave precisely the same safety allowance.
        while baseline_budget.spend(1).is_ok() {
            assert!(route_budget.spend(1).is_ok());
        }
        assert!(route_budget.spend(1).is_err());
    }
}
