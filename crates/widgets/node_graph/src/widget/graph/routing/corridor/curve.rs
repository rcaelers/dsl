//! Conservative cubic/rectangle separation, independent of interaction flattening.

use egui::{Pos2, Rect};

use super::super::{RouteFailure, WorkBudget};

#[derive(Clone, Copy)]
struct Interval {
    low: f64,
    high: f64,
}

impl Interval {
    fn exact(value: f32) -> Self {
        Self {
            low: value as f64,
            high: value as f64,
        }
    }

    fn midpoint(self, other: Self) -> Self {
        // f32-origin coordinates cannot overflow f64 addition. Round outward at
        // both operations, including subnormal interval bounds around exact zero.
        Self {
            low: ((self.low + other.low).next_down() * 0.5).next_down(),
            high: ((self.high + other.high).next_up() * 0.5).next_up(),
        }
    }
}

#[derive(Clone, Copy)]
struct Point {
    x: Interval,
    y: Interval,
}

impl Point {
    fn midpoint(self, other: Self) -> Self {
        Self {
            x: self.x.midpoint(other.x),
            y: self.y.midpoint(other.y),
        }
    }
}

pub(crate) fn cubic_clear(
    points: [Pos2; 4],
    obstacles: &[Rect],
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    budget.spend(obstacles.len())?;
    if points.iter().any(|p| !p.is_finite())
        || obstacles
            .iter()
            .any(|r| !r.min.is_finite() || !r.max.is_finite() || r.is_negative())
    {
        return Err(RouteFailure::InvalidGeometry);
    }
    let hull = points.map(|p| Point {
        x: Interval::exact(p.x),
        y: Interval::exact(p.y),
    });
    for obstacle in obstacles {
        if !separated(hull, *obstacle, 0, budget)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn separated(
    points: [Point; 4],
    obstacle: Rect,
    depth: u8,
    budget: &mut WorkBudget,
) -> Result<bool, RouteFailure> {
    budget.spend(1)?;
    let min_x = points.iter().map(|p| p.x.low).fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|p| p.x.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.y.low).fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|p| p.y.high)
        .fold(f64::NEG_INFINITY, f64::max);
    if max_x < obstacle.min.x as f64
        || min_x > obstacle.max.x as f64
        || max_y < obstacle.min.y as f64
        || min_y > obstacle.max.y as f64
    {
        return Ok(true);
    }
    // Known endpoint contact is a collision. Ambiguous contact at the depth bound
    // is also rejected; uncertainty must never become a safety certificate.
    let inside = |p: Point| {
        p.x.low >= obstacle.min.x as f64
            && p.x.high <= obstacle.max.x as f64
            && p.y.low >= obstacle.min.y as f64
            && p.y.high <= obstacle.max.y as f64
    };
    if inside(points[0]) || inside(points[3]) || depth == 12 {
        return Ok(false);
    }
    let a = points[0].midpoint(points[1]);
    let b = points[1].midpoint(points[2]);
    let c = points[2].midpoint(points[3]);
    let d = a.midpoint(b);
    let e = b.midpoint(c);
    let middle = d.midpoint(e);
    Ok(
        separated([points[0], a, d, middle], obstacle, depth + 1, budget)?
            && separated([middle, e, c, points[3]], obstacle, depth + 1, budget)?,
    )
}

#[cfg(test)]
mod curve_tests {
    use super::*;

    fn arch() -> [Pos2; 4] {
        [
            Pos2::ZERO,
            Pos2::new(0.0, 100.0),
            Pos2::new(100.0, 100.0),
            Pos2::new(100.0, 0.0),
        ]
    }

    #[test]
    fn subdivision_proves_clearance_when_the_parent_box_overlaps() {
        let obstacle = Rect::from_min_max(Pos2::new(40.0, 0.0), Pos2::new(60.0, 20.0));
        assert!(Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0)).intersects(obstacle));
        assert!(cubic_clear(arch(), &[obstacle], &mut WorkBudget::new(10000)).unwrap());
    }

    #[test]
    fn collision_tangency_and_endpoint_contact_are_not_accepted() {
        for obstacle in [
            Rect::from_min_max(Pos2::new(49.0, 74.0), Pos2::new(51.0, 76.0)),
            Rect::from_min_max(Pos2::new(49.0, 75.0), Pos2::new(51.0, 76.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        ] {
            assert!(!cubic_clear(arch(), &[obstacle], &mut WorkBudget::new(10000)).unwrap());
        }
    }

    #[test]
    fn exhausted_work_and_nonfinite_geometry_are_explicit() {
        assert!(matches!(
            cubic_clear(
                arch(),
                &[Rect::from_min_max(
                    Pos2::new(200.0, 200.0),
                    Pos2::new(300.0, 300.0)
                )],
                &mut WorkBudget::new(0)
            ),
            Err(RouteFailure::WorkLimit)
        ));
        let mut points = arch();
        points[1].x = f32::NAN;
        assert!(matches!(
            cubic_clear(points, &[], &mut WorkBudget::new(100)),
            Err(RouteFailure::InvalidGeometry)
        ));
    }
}
