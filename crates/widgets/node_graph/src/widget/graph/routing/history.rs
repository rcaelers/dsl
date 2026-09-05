//! Revalidation of immutable checked geometry against changed, non-endpoint bodies.

use egui::Rect;

use super::contract::{RouteConfig, RouteFailure, WorkBudget};
use super::corridor::{clear, cubic_clear, expand_obstacle};
use super::geometry::{PathSegment, WirePath};

/// The caller proves endpoints, configuration, and all other obstacles unchanged.
/// No own-node exemption applies to these changed non-endpoint obstacles.
pub(crate) fn avoids_changed_obstacles(
    path: &WirePath,
    changes: &[(Rect, Rect)],
    config: &RouteConfig,
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    for &(old, new) in changes {
        budget.spend(1)?;
        let old = expand_obstacle(old, config)?;
        let new = expand_obstacle(new, config)?;
        if !path.bounds().intersects(old) && !path.bounds().intersects(new) {
            continue;
        }
        // Old extents identify dependencies; only the current obstacle can collide.
        // Use exact line / conservative cubic proof, never interaction flattening.
        for segment in path.segments() {
            budget.spend(1)?;
            let safe = match segment {
                PathSegment::Line(points) => clear(*points, &[new], None, budget)?,
                PathSegment::Cubic(points) => cubic_clear(*points, &[new], budget)?,
            };
            if !safe {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod history_tests {
    use egui::Pos2;

    use super::*;

    #[test]
    fn changed_obstacles_use_curve_proof_not_just_control_hulls() {
        let path = WirePath::new(
            vec![PathSegment::Cubic([
                Pos2::ZERO,
                Pos2::new(0.0, 100.0),
                Pos2::new(100.0, 100.0),
                Pos2::new(100.0, 0.0),
            ])],
            0.5,
        );
        let config = RouteConfig {
            clearance_x: 0.0,
            clearance_y: 0.0,
            ..RouteConfig::default()
        };
        let old = Rect::from_min_max(Pos2::new(200.0, 200.0), Pos2::new(210.0, 210.0));
        for (new, expected) in [
            (
                Rect::from_min_max(Pos2::new(40.0, 0.0), Pos2::new(60.0, 20.0)),
                true,
            ),
            (
                Rect::from_min_max(Pos2::new(49.0, 75.0), Pos2::new(51.0, 76.0)),
                false,
            ),
        ] {
            assert!(path.bounds().intersects(new));
            assert_eq!(
                avoids_changed_obstacles(
                    &path,
                    &[(old, new)],
                    &config,
                    &mut WorkBudget::new(10_000)
                ),
                Ok(expected)
            );
        }
    }

    #[test]
    fn changed_invalid_geometry_contact_and_budget_exhaustion_cannot_certify_reuse() {
        let path = WirePath::new(
            vec![PathSegment::Line([Pos2::ZERO, Pos2::new(100.0, 0.0)])],
            0.5,
        );
        let config = RouteConfig::default();
        let old = Rect::from_min_max(Pos2::new(200.0, 200.0), Pos2::new(210.0, 210.0));
        let tangent = Rect::from_min_max(Pos2::new(30.0, 16.0), Pos2::new(40.0, 20.0));
        assert_eq!(
            avoids_changed_obstacles(&path, &[(old, tangent)], &config, &mut WorkBudget::new(100)),
            Ok(false)
        );
        assert_eq!(
            avoids_changed_obstacles(&path, &[(old, old)], &config, &mut WorkBudget::new(0)),
            Err(RouteFailure::WorkLimit)
        );
        let invalid = Rect::from_min_max(Pos2::new(f32::NAN, 0.0), Pos2::new(10.0, 10.0));
        assert_eq!(
            avoids_changed_obstacles(&path, &[(old, invalid)], &config, &mut WorkBudget::new(100)),
            Err(RouteFailure::InvalidGeometry)
        );
    }
}
