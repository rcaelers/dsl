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
    clear_indexed(
        segment,
        obstacles.iter().copied().enumerate(),
        exempt,
        budget,
    )
}

/// A conservative broad phase for repeated segment checks inside one known envelope.
/// Original indexes are retained so endpoint exemptions never change identity.
pub(crate) struct ObstacleSubset {
    bounds: Rect,
    obstacles: Vec<(usize, Rect)>,
}

impl ObstacleSubset {
    pub(crate) fn new(
        obstacles: &[Rect],
        bounds: Rect,
        budget: &mut WorkBudget,
    ) -> Result<Self, RouteFailure> {
        if !bounds.is_finite() || bounds.is_negative() {
            return Err(RouteFailure::InvalidGeometry);
        }
        budget.spend(obstacles.len())?;
        let mut selected = Vec::new();
        for (index, &obstacle) in obstacles.iter().enumerate() {
            if !valid_rect(obstacle) {
                return Err(RouteFailure::InvalidGeometry);
            }
            if obstacle.intersects(bounds) {
                selected.push((index, obstacle));
            }
        }
        Ok(Self {
            bounds,
            obstacles: selected,
        })
    }

    pub(crate) fn clear(
        &self,
        segment: [Pos2; 2],
        exempt: Option<usize>,
        budget: &mut WorkBudget,
    ) -> Result<bool, RouteFailure> {
        if !segment.iter().all(|p| p.is_finite())
            || !self
                .bounds
                .contains_rect(Rect::from_two_pos(segment[0], segment[1]))
        {
            return Err(RouteFailure::InvalidGeometry);
        }
        clear_indexed(segment, self.obstacles.iter().copied(), exempt, budget)
    }
}

fn clear_indexed(
    segment: [Pos2; 2],
    obstacles: impl Iterator<Item = (usize, Rect)>,
    exempt: Option<usize>,
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    if !segment.iter().all(|point| point.is_finite())
        || (segment[0].x != segment[1].x && segment[0].y != segment[1].y)
    {
        return Err(RouteFailure::InvalidGeometry);
    }
    let bounds = Rect::from_two_pos(segment[0], segment[1]);
    for (index, obstacle) in obstacles {
        budget.spend(1)?;
        if Some(index) != exempt && bounds.intersects(obstacle) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod subset_tests {
    use super::*;

    fn rect(x: f32, y: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::splat(10.0))
    }

    #[test]
    fn subset_matches_full_checks_including_contact_and_original_exemption_indexes() {
        let obstacles = [
            rect(1000.0, 1000.0),
            rect(20.0, 20.0),
            rect(-1000.0, -1000.0),
            rect(70.0, 60.0),
        ];
        let bounds = Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0));
        let subset = ObstacleSubset::new(&obstacles, bounds, &mut WorkBudget::new(100)).unwrap();
        for coordinate in [0.0, 19.0, 20.0, 25.0, 30.0, 31.0, 60.0, 70.0, 80.0, 100.0] {
            for segment in [
                [Pos2::new(0.0, coordinate), Pos2::new(100.0, coordinate)],
                [Pos2::new(coordinate, 0.0), Pos2::new(coordinate, 100.0)],
                [Pos2::new(coordinate, coordinate); 2],
            ] {
                for exempt in [None, Some(0), Some(1), Some(2), Some(3)] {
                    assert_eq!(
                        subset.clear(segment, exempt, &mut WorkBudget::new(100)),
                        clear(segment, &obstacles, exempt, &mut WorkBudget::new(100))
                    );
                }
            }
        }
        assert!(
            !subset
                .clear(
                    [Pos2::new(0.0, 20.0), Pos2::new(100.0, 20.0)],
                    None,
                    &mut WorkBudget::new(100)
                )
                .unwrap()
        );
    }

    #[test]
    fn subset_rejects_out_of_envelope_invalid_and_unbudgeted_checks() {
        let bounds = rect(0.0, 0.0);
        let obstacles = [rect(50.0, 50.0)];
        assert!(matches!(
            ObstacleSubset::new(&obstacles, bounds, &mut WorkBudget::new(0)),
            Err(RouteFailure::WorkLimit)
        ));
        let subset = ObstacleSubset::new(&obstacles, bounds, &mut WorkBudget::new(1)).unwrap();
        for segment in [
            [Pos2::ZERO, Pos2::new(11.0, 0.0)],
            [Pos2::ZERO, Pos2::new(1.0, 1.0)],
            [Pos2::ZERO, Pos2::new(f32::NAN, 0.0)],
        ] {
            assert_eq!(
                subset.clear(segment, None, &mut WorkBudget::new(100)),
                Err(RouteFailure::InvalidGeometry)
            );
        }
        let invalid = Rect::from_min_max(Pos2::new(f32::NAN, 50.0), Pos2::new(60.0, 60.0));
        assert!(matches!(
            ObstacleSubset::new(&[invalid], bounds, &mut WorkBudget::new(100)),
            Err(RouteFailure::InvalidGeometry)
        ));
        let subset =
            ObstacleSubset::new(&[rect(5.0, 5.0)], bounds, &mut WorkBudget::new(1)).unwrap();
        assert_eq!(
            subset.clear(
                [Pos2::ZERO, Pos2::new(10.0, 0.0)],
                None,
                &mut WorkBudget::new(0)
            ),
            Err(RouteFailure::WorkLimit)
        );
    }

    #[test]
    fn repeated_checks_spend_work_only_on_possible_colliders() {
        let obstacles: Vec<_> = (0..500).map(|i| rect(i as f32 * 100.0, 50.0)).collect();
        let bounds = Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0));
        let mut budget = WorkBudget::new(600);
        let subset = ObstacleSubset::new(&obstacles, bounds, &mut budget).unwrap();
        for _ in 0..100 {
            assert!(
                subset
                    .clear([Pos2::ZERO, Pos2::new(50.0, 0.0)], None, &mut budget)
                    .unwrap()
            );
        }
        assert!(budget.spend(1).is_err());
    }
}
