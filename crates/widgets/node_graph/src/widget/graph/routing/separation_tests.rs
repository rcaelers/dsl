use egui::{Pos2, Rect};

use super::contract::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::corridor::{Channels, parallel_overlap};
use super::geometry::{PathSegment, WirePath};
use super::separation::{separate_route, shares_run};

fn line(a: [f32; 2], b: [f32; 2]) -> WirePath {
    WirePath::new(vec![PathSegment::Line([Pos2::from(a), Pos2::from(b)])], 0.5)
}

#[test]
fn overlaps_require_a_run_not_a_crossing_or_junction_point() {
    let horizontal = line([0.0, 20.0], [100.0, 20.0]);
    for (other, expected) in [
        (line([30.0, 20.0], [80.0, 20.0]), true),
        (line([80.0, 20.0], [30.0, 20.0]), true),
        (line([100.0, 20.0], [200.0, 20.0]), false),
        (line([50.0, 0.0], [50.0, 40.0]), false),
        (line([20.0, 20.0], [20.0, 20.0]), false),
        (line([0.0, 28.0], [100.0, 28.0]), false),
    ] {
        assert_eq!(
            shares_run(&horizontal, &other, &mut WorkBudget::new(100)).unwrap(),
            expected
        );
    }
    let cubic = WirePath::new(
        vec![PathSegment::Cubic([
            Pos2::new(10.0, 20.0),
            Pos2::new(30.0, 20.0),
            Pos2::new(60.0, 20.0),
            Pos2::new(90.0, 20.0),
        ])],
        0.5,
    );
    assert!(shares_run(&horizontal, &cubic, &mut WorkBudget::new(100)).unwrap());
    assert!(matches!(
        shares_run(&horizontal, &cubic, &mut WorkBudget::new(0)),
        Err(RouteFailure::WorkLimit)
    ));
}

#[test]
fn identical_nonlinear_arcs_are_shared_runs_in_either_direction() {
    let points = [
        Pos2::new(0.0, 0.0),
        Pos2::new(20.0, 0.0),
        Pos2::new(80.0, 100.0),
        Pos2::new(100.0, 100.0),
    ];
    let a = WirePath::new(vec![PathSegment::Cubic(points)], 0.5);
    let mut reversed = points;
    reversed.reverse();
    for points in [points, reversed] {
        let b = WirePath::new(vec![PathSegment::Cubic(points)], 0.5);
        assert!(shares_run(&a, &b, &mut WorkBudget::new(100)).unwrap());
    }
}

#[test]
fn reserved_runs_allow_crossings_but_choose_a_spaced_parallel_track() {
    let config = RouteConfig::default();
    let start = Pos2::new(0.0, 0.0);
    let end = Pos2::new(100.0, 100.0);
    let occupied = [Pos2::new(100.0, 20.0), Pos2::new(100.0, 80.0)];
    let mut budget = WorkBudget::new(config.max_work);
    let channels =
        Channels::avoiding_runs(start, end, &[], &[occupied], &config, &mut budget).unwrap();
    assert!(
        channels
            .run_clear([Pos2::new(90.0, 50.0), Pos2::new(110.0, 50.0)], &mut budget)
            .unwrap()
    );
    assert!(
        !channels
            .run_clear([Pos2::new(100.0, 0.0), end], &mut budget)
            .unwrap()
    );
    assert!(
        !channels
            .run_clear(
                [Pos2::new(99.99, 0.0), Pos2::new(99.99, 100.0)],
                &mut budget
            )
            .unwrap()
    );
    let path = channels
        .find(start, end, &[], &config, true, &mut budget)
        .unwrap();
    assert_eq!(path.first(), Some(&start));
    assert_eq!(path.last(), Some(&end));
    for pair in path.windows(2) {
        assert!(!parallel_overlap(
            [pair[0], pair[1]],
            occupied,
            config.lane_spacing
        ));
    }
}

#[test]
fn separated_route_does_not_reverse_at_overlapping_escape_extents() {
    let nodes = [
        Rect::from_min_max(Pos2::new(200.0, 300.0), Pos2::new(300.0, 400.0)),
        Rect::from_min_max(Pos2::new(400.0, 0.0), Pos2::new(500.0, 150.0)),
    ];
    let config = RouteConfig {
        escape: 54.0,
        ..RouteConfig::default()
    };
    let other = line([354.0, 100.0], [354.0, 250.0]);
    for zoom in [0.5, 1.0, 2.0] {
        let path = separate_route(
            RouteInput {
                nodes: &nodes,
                source: PortGeometry {
                    obstacle: 0,
                    position: nodes[0].right_center(),
                    side: PortSide::Right,
                },
                target: PortGeometry {
                    obstacle: 1,
                    position: nodes[1].left_center(),
                    side: PortSide::Left,
                },
            },
            &[(&other, false)],
            &config,
            zoom,
            &mut WorkBudget::new(config.max_work),
        )
        .unwrap();
        assert!(!shares_run(&path, &other, &mut WorkBudget::new(config.max_work)).unwrap());
        let mut points = vec![nodes[0].right_center()];
        for segment in path.segments() {
            match segment {
                PathSegment::Line(p) => points.extend_from_slice(&p[1..]),
                PathSegment::Cubic(p) => points.extend_from_slice(&p[1..]),
            }
        }
        assert_eq!(points.last(), Some(&nodes[1].left_center()));
        for turn in points.windows(3) {
            assert!(
                (turn[1] - turn[0]).dot(turn[2] - turn[1]) >= 0.0,
                "protruding turn: {turn:?}"
            );
        }
    }
}

#[test]
fn separated_route_keeps_checked_node_clearance_escapes_and_wire_geometry() {
    let nodes = [
        Rect::from_min_max(Pos2::new(0.0, 300.0), Pos2::new(100.0, 400.0)),
        Rect::from_min_max(Pos2::new(400.0, 0.0), Pos2::new(500.0, 150.0)),
    ];
    let source = PortGeometry {
        obstacle: 0,
        position: Pos2::new(100.0, 350.0),
        side: PortSide::Right,
    };
    let target = PortGeometry {
        obstacle: 1,
        position: Pos2::new(400.0, 90.0),
        side: PortSide::Left,
    };
    let config = RouteConfig {
        corner_radius: 0.0,
        ..RouteConfig::default()
    };
    let other = line([370.0, 20.0], [370.0, 300.0]);
    let path = separate_route(
        RouteInput {
            nodes: &nodes,
            source,
            target,
        },
        &[(&other, false)],
        &config,
        1.0,
        &mut WorkBudget::new(config.max_work),
    )
    .unwrap();
    assert!(!shares_run(&path, &other, &mut WorkBudget::new(config.max_work)).unwrap());
    let mut previous = source.position;
    for (index, segment) in path.segments().iter().enumerate() {
        let PathSegment::Line([a, b]) = segment else {
            panic!("rectilinear retry");
        };
        assert_eq!(*a, previous);
        assert_eq!(path.distance(a.lerp(*b, 0.5)), 0.0);
        previous = *b;
        for (node, body) in nodes.iter().enumerate() {
            if (index == 0 && node == source.obstacle)
                || (index + 1 == path.segments().len() && node == target.obstacle)
            {
                continue;
            }
            let body = body.expand2(egui::vec2(config.clearance_x, config.clearance_y));
            assert!(
                a.x.max(b.x) < body.min.x
                    || a.x.min(b.x) > body.max.x
                    || a.y.max(b.y) < body.min.y
                    || a.y.min(b.y) > body.max.y
            );
        }
    }
    assert_eq!(previous, target.position);
    // A peer in the same compatible bundle cannot acquire a new crossing.
    let peer = line([130.0, 200.0], [370.0, 200.0]);
    let protected = separate_route(
        RouteInput {
            nodes: &nodes,
            source,
            target,
        },
        &[(&peer, true)],
        &config,
        1.0,
        &mut WorkBudget::new(config.max_work),
    )
    .unwrap();
    let peer_bounds = Rect::from_min_max(Pos2::new(130.0, 200.0), Pos2::new(370.0, 200.0))
        .expand(config.lane_spacing);
    for segment in protected.segments() {
        let PathSegment::Line(p) = segment else {
            panic!("protected retry stays rectilinear");
        };
        assert!(!Rect::from_two_pos(p[0], p[1]).intersects(peer_bounds));
    }
    // Reserving another signal on a mandatory escape is explicitly unroutable.
    let blocked = line([100.0, 350.0], [150.0, 350.0]);
    assert!(matches!(
        separate_route(
            RouteInput {
                nodes: &nodes,
                source,
                target
            },
            &[(&blocked, false)],
            &config,
            1.0,
            &mut WorkBudget::new(config.max_work),
        ),
        Err(RouteFailure::NoCorridor)
    ));
    assert!(matches!(
        separate_route(
            RouteInput {
                nodes: &nodes,
                source,
                target
            },
            &[(&other, false)],
            &config,
            1.0,
            &mut WorkBudget::new(0)
        ),
        Err(RouteFailure::WorkLimit)
    ));
}
