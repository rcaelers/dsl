use egui::{Pos2, Rect, Vec2};

use super::contract::{PortGeometry, PortSide, RouteConfig, RouteInput, WorkBudget};
use super::corridor::{cubic_clear, escape, expanded};
use super::geometry::{PathSegment, WirePath};
use super::individual::route_with_budget;
use super::individual_quality::improve_route;
use super::smoothing::smooth_route;

fn nodes() -> Vec<Rect> {
    vec![
        Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0)),
        Rect::from_min_max(Pos2::new(500.0, 0.0), Pos2::new(550.0, 100.0)),
        Rect::from_min_max(Pos2::new(200.0, 0.0), Pos2::new(300.0, 100.0)),
    ]
}

fn input(nodes: &[Rect]) -> RouteInput<'_> {
    RouteInput {
        nodes,
        source: PortGeometry {
            obstacle: 0,
            side: PortSide::Right,
            position: Pos2::new(50.0, 50.0),
        },
        target: PortGeometry {
            obstacle: 1,
            side: PortSide::Left,
            position: Pos2::new(500.0, 50.0),
        },
    }
}

fn checked() -> WirePath {
    let points = [
        Pos2::new(50.0, 50.0),
        Pos2::new(80.0, 50.0),
        Pos2::new(80.0, 150.0),
        Pos2::new(470.0, 150.0),
        Pos2::new(470.0, 50.0),
        Pos2::new(500.0, 50.0),
    ];
    WirePath::new(
        points
            .windows(2)
            .map(|p| PathSegment::Line([p[0], p[1]]))
            .collect(),
        0.5,
    )
}

fn evaluate(p: [Pos2; 4], t: f32) -> Pos2 {
    let a = p[0].lerp(p[1], t);
    let b = p[1].lerp(p[2], t);
    let c = p[2].lerp(p[3], t);
    a.lerp(b, t).lerp(b.lerp(c, t), t)
}

#[test]
fn rounded_corners_keep_escapes_clearance_and_aligned_tangents() {
    let nodes = nodes();
    let config = RouteConfig {
        clearance_x: 25.0,
        clearance_y: 19.0,
        ..RouteConfig::default()
    };
    let path = smooth_route(
        input(&nodes),
        checked(),
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_eq!(
        format!("{:?}", path.segments().first()),
        format!("{:?}", checked().segments().first())
    );
    assert_eq!(
        format!("{:?}", path.segments().last()),
        format!("{:?}", checked().segments().last())
    );
    let obstacles: Vec<_> = nodes
        .iter()
        .map(|r| r.expand2(Vec2::new(config.clearance_x, config.clearance_y)))
        .collect();
    let mut curves = 0;
    for (i, segment) in path.segments().iter().enumerate() {
        if let PathSegment::Cubic(p) = segment {
            curves += 1;
            assert!(cubic_clear(*p, &obstacles, &mut WorkBudget::new(10000)).unwrap());
            let PathSegment::Line(before) = path.segments()[i - 1] else {
                panic!("incoming tangent")
            };
            let PathSegment::Line(after) = path.segments()[i + 1] else {
                panic!("outgoing tangent")
            };
            assert_eq!(before[1], p[0]);
            assert_eq!(after[0], p[3]);
            for (a, b) in [
                (before[1] - before[0], p[1] - p[0]),
                (after[1] - after[0], p[3] - p[2]),
            ] {
                assert_eq!(a.x * b.y - a.y * b.x, 0.0);
                assert!(a.dot(b) > 0.0);
            }
            for t in 0..=1000 {
                assert!(
                    obstacles
                        .iter()
                        .all(|r| !r.contains(evaluate(*p, t as f32 / 1000.0)))
                );
            }
        }
    }
    assert_eq!(curves, 2);
}

#[test]
fn gestures_use_the_rounded_geometry_at_each_zoom() {
    let nodes = nodes();
    let config = RouteConfig::default();
    for zoom in [0.2, 1.0, 3.0] {
        let path = smooth_route(
            input(&nodes),
            checked(),
            &config,
            zoom,
            &mut WorkBudget::new(config.max_smoothing_work),
        );
        assert!(path.distance(Pos2::new(80.0, 150.0)) > 1.0);
        for segment in path.segments() {
            if let PathSegment::Cubic(p) = segment {
                let middle = evaluate(*p, 0.5);
                assert!(path.intersects_segment([
                    middle - Vec2::new(2.0, 0.0),
                    middle + Vec2::new(2.0, 0.0)
                ]));
                for t in 0..=1000 {
                    assert!(path.distance(evaluate(*p, t as f32 / 1000.0)) * zoom <= 0.501);
                }
            }
        }
    }
}

#[test]
fn tight_corners_and_disabled_quality_keep_the_exact_checked_path() {
    let mut nodes = nodes();
    nodes.extend([
        Rect::from_min_max(Pos2::new(80.001, 120.0), Pos2::new(100.0, 149.999)),
        Rect::from_min_max(Pos2::new(450.0, 120.0), Pos2::new(469.999, 149.999)),
    ]);
    let config = RouteConfig {
        clearance_x: 0.0,
        clearance_y: 0.0,
        ..RouteConfig::default()
    };
    let original = format!("{:?}", checked().segments());
    let result = smooth_route(
        input(&nodes),
        checked(),
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_eq!(original, format!("{:?}", result.segments()));
    for radius in [0.0, f32::NAN] {
        let config = RouteConfig {
            corner_radius: radius,
            ..config
        };
        let result = smooth_route(
            input(&nodes),
            checked(),
            &config,
            1.0,
            &mut WorkBudget::new(config.max_smoothing_work),
        );
        assert_eq!(original, format!("{:?}", result.segments()));
    }
}

#[test]
fn exhausted_quality_work_never_loses_the_checked_route() {
    let nodes = nodes();
    let config = RouteConfig::default();
    let original = format!("{:?}", checked().segments());
    let mut outcomes = [false; 2];
    for work in 0..100 {
        let result = smooth_route(
            input(&nodes),
            checked(),
            &config,
            1.0,
            &mut WorkBudget::new(work),
        );
        let curves = result
            .segments()
            .iter()
            .filter(|s| matches!(s, PathSegment::Cubic(_)))
            .count();
        if curves == 0 {
            assert_eq!(original, format!("{:?}", result.segments()));
            outcomes[0] = true;
        } else {
            assert_eq!(curves, 2);
            outcomes[1] = true;
        }
    }
    assert!(outcomes.into_iter().all(|seen| seen));
}

#[test]
fn reserved_endpoint_transitions_preserve_minimum_escapes_and_clear_all_nodes() {
    let config = RouteConfig {
        clearance_x: 25.0,
        clearance_y: 19.0,
        ..RouteConfig::default()
    };
    let nodes = nodes();
    for mirrored in [false, true] {
        let nodes: Vec<_> = nodes
            .iter()
            .map(|r| {
                if mirrored {
                    Rect::from_min_max(Pos2::new(-r.max.x, r.min.y), Pos2::new(-r.min.x, r.max.y))
                } else {
                    *r
                }
            })
            .collect();
        let geometry = || {
            let mut input = input(&nodes);
            if mirrored {
                input.source.position.x = -input.source.position.x;
                input.target.position.x = -input.target.position.x;
                input.source.side = PortSide::Left;
                input.target.side = PortSide::Right;
            }
            input
        };
        for zoom in [0.5, 1.0, 1.7] {
            let baseline =
                route_with_budget(geometry(), &config, &mut WorkBudget::new(config.max_work))
                    .unwrap();
            let path = improve_route(
                geometry(),
                baseline,
                &config,
                zoom,
                &mut WorkBudget::new(config.max_smoothing_work),
            );
            let mut budget = WorkBudget::new(config.max_work);
            let obstacles = expanded(&geometry(), &config, &mut budget).unwrap();
            let source =
                escape(geometry().source, &nodes, &obstacles, &config, &mut budget).unwrap();
            let target =
                escape(geometry().target, &nodes, &obstacles, &config, &mut budget).unwrap();
            let segments = path.segments();
            let PathSegment::Line(first) = segments[0] else {
                panic!("straight source escape")
            };
            let PathSegment::Line(last) = segments[segments.len() - 1] else {
                panic!("straight target escape")
            };
            assert!(Rect::from_two_pos(first[0], first[1]).contains(source));
            assert!(Rect::from_two_pos(last[0], last[1]).contains(target));
            let PathSegment::Cubic(start) = segments[1] else {
                panic!("smooth source transition")
            };
            let PathSegment::Cubic(end) = segments[segments.len() - 2] else {
                panic!("smooth target transition")
            };
            assert_eq!(first[1], start[0]);
            assert_eq!(start[0].y, start[1].y);
            assert_eq!(last[0], end[3]);
            assert_eq!(end[2].y, end[3].y);
            for segment in segments {
                if let PathSegment::Cubic(curve) = segment {
                    assert!(cubic_clear(*curve, &obstacles, &mut budget).unwrap());
                    for t in 0..=100 {
                        assert!(path.distance(evaluate(*curve, t as f32 / 100.0)) * zoom <= 0.501);
                    }
                }
            }
        }
    }
}

#[test]
fn blocked_endpoint_reservation_and_zero_quality_keep_the_safe_route() {
    let config = RouteConfig::default();
    let mut nodes = nodes();
    // Ordinary escape ends at x=80; the obstacle begins at expanded x=90,
    // blocking only the extended escape to x=104.
    nodes.push(Rect::from_min_max(
        Pos2::new(110.0, 40.0),
        Pos2::new(120.0, 60.0),
    ));
    let baseline = || {
        route_with_budget(
            input(&nodes),
            &config,
            &mut WorkBudget::new(config.max_work),
        )
        .unwrap()
    };
    let expected = format!("{:?}", baseline().segments());
    let disabled = improve_route(
        input(&nodes),
        baseline(),
        &config,
        1.0,
        &mut WorkBudget::new(0),
    );
    assert_eq!(format!("{:?}", disabled.segments()), expected);
    let path = improve_route(
        input(&nodes),
        baseline(),
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_eq!(
        format!("{:?}", path.segments().first()),
        format!("{:?}", baseline().segments().first())
    );
    let obstacles = expanded(
        &input(&nodes),
        &config,
        &mut WorkBudget::new(config.max_work),
    )
    .unwrap();
    for segment in path.segments() {
        if let PathSegment::Cubic(curve) = segment {
            assert!(
                cubic_clear(*curve, &obstacles, &mut WorkBudget::new(config.max_work)).unwrap()
            );
        }
    }
}

#[test]
fn short_straight_connection_does_not_request_longer_escapes() {
    let config = RouteConfig::default();
    let nodes = vec![
        Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0)),
        Rect::from_min_max(Pos2::new(120.0, 0.0), Pos2::new(170.0, 100.0)),
    ];
    let geometry = || {
        let mut input = input(&nodes);
        input.target.position.x = 120.0;
        input
    };
    let baseline =
        route_with_budget(geometry(), &config, &mut WorkBudget::new(config.max_work)).unwrap();
    let expected = format!("{:?}", baseline.segments());
    let path = improve_route(
        geometry(),
        baseline,
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_eq!(format!("{:?}", path.segments()), expected);
}
