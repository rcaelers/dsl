use egui::{Pos2, Rect};

use super::super::PathSegment;
use super::contract::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::obstacle::clear;
use super::router::route;
use super::search::Channels;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), egui::vec2(w, h))
}

fn endpoints(nodes: &[Rect]) -> RouteInput<'_> {
    RouteInput {
        nodes,
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
    }
}

fn checked_points(input: RouteInput<'_>) -> Vec<Pos2> {
    let config = RouteConfig::default();
    let source = input.source;
    let target = input.target;
    let nodes = input.nodes;
    let path = route(input, &config).unwrap();
    let mut points = vec![source.position];
    for (index, segment) in path.segments().iter().enumerate() {
        let PathSegment::Line([a, b]) = segment else {
            panic!("checked line path")
        };
        assert_eq!(points.last(), Some(a));
        assert!(a.x == b.x || a.y == b.y);
        for (node, body) in nodes.iter().enumerate() {
            if (index == 0 && node == source.obstacle)
                || (index + 1 == path.segments().len() && node == target.obstacle)
            {
                continue;
            }
            let obstacle = body.expand2(egui::vec2(config.clearance_x, config.clearance_y));
            // Independent analytic separation oracle, including closed-boundary contact.
            assert!(
                a.x.max(b.x) < obstacle.min.x
                    || a.x.min(b.x) > obstacle.max.x
                    || a.y.max(b.y) < obstacle.min.y
                    || a.y.min(b.y) > obstacle.max.y,
                "segment {a:?} -> {b:?} intersects {obstacle:?}"
            );
        }
        points.push(*b);
    }
    assert_eq!(points.last(), Some(&target.position));
    assert_eq!(points[0].y, points[1].y);
    assert_eq!(points[points.len() - 1].y, points[points.len() - 2].y);
    assert!(points[1].x > points[0].x);
    assert!(points[points.len() - 2].x < points[points.len() - 1].x);
    points
}

#[test]
fn straight_and_obstructed_forward_routes_have_checked_escapes() {
    let base = [rect(0.0, 0.0, 40.0, 80.0), rect(400.0, 0.0, 40.0, 80.0)];
    let points = checked_points(endpoints(&base));
    assert!(points.iter().all(|point| point.y == 40.0));
    let nodes = [base[0], base[1], rect(160.0, -20.0, 80.0, 130.0)];
    let points = checked_points(endpoints(&nodes));
    assert!(
        points
            .iter()
            .any(|point| point.y < -36.0 || point.y > 126.0)
    );
    assert!(points.windows(2).all(|p| p[1].x >= p[0].x));
}

#[test]
fn backward_equal_x_and_same_node_routes_keep_endpoint_directions() {
    checked_points(endpoints(&[
        rect(400.0, 0.0, 40.0, 80.0),
        rect(0.0, 100.0, 40.0, 80.0),
    ]));
    checked_points(endpoints(&[
        rect(0.0, 0.0, 40.0, 80.0),
        rect(0.0, 200.0, 40.0, 80.0),
    ]));
    let nodes = [rect(0.0, 0.0, 40.0, 80.0)];
    checked_points(RouteInput {
        nodes: &nodes,
        source: PortGeometry {
            obstacle: 0,
            position: nodes[0].right_center(),
            side: PortSide::Right,
        },
        target: PortGeometry {
            obstacle: 0,
            position: nodes[0].left_center(),
            side: PortSide::Left,
        },
    });
}

#[test]
fn too_short_escape_is_extended_but_another_body_is_not_exempted() {
    let nodes = [rect(0.0, 0.0, 40.0, 80.0), rect(400.0, 0.0, 40.0, 80.0)];
    let config = RouteConfig {
        escape: 0.0,
        ..RouteConfig::default()
    };
    let path = route(endpoints(&nodes), &config).unwrap();
    let PathSegment::Line([_, escape]) = path.segments()[0] else {
        panic!()
    };
    assert!(escape.x > 60.0);
    let blocked = [nodes[0], nodes[1], rect(45.0, 0.0, 10.0, 80.0)];
    assert!(matches!(
        route(endpoints(&blocked), &config),
        Err(RouteFailure::BlockedEscape)
    ));
    let covered = [nodes[0], nodes[1], rect(20.0, 0.0, 100.0, 80.0)];
    assert!(matches!(
        route(endpoints(&covered), &config),
        Err(RouteFailure::BlockedEscape)
    ));
}

#[test]
fn forward_connection_can_escape_a_cul_de_sac_by_falling_back() {
    let nodes = [
        rect(0.0, 0.0, 40.0, 80.0),
        rect(500.0, 0.0, 40.0, 80.0),
        rect(-100.0, -80.0, 350.0, 20.0),
        rect(-100.0, 140.0, 350.0, 20.0),
        rect(230.0, -80.0, 20.0, 240.0),
    ];
    let points = checked_points(endpoints(&nodes));
    assert!(points.iter().any(|point| point.x < -120.0));
}

#[test]
fn invalid_input_and_budget_limits_are_distinct_failures() {
    let mut nodes = [rect(0.0, 0.0, 40.0, 80.0), rect(400.0, 0.0, 40.0, 80.0)];
    for config in [
        RouteConfig {
            clearance_x: -1.0,
            ..RouteConfig::default()
        },
        RouteConfig {
            safety: 0.0,
            ..RouteConfig::default()
        },
        RouteConfig {
            escape: f32::NAN,
            ..RouteConfig::default()
        },
    ] {
        assert!(matches!(
            route(endpoints(&nodes), &config),
            Err(RouteFailure::InvalidGeometry)
        ));
    }
    for config in [
        RouteConfig {
            max_work: 0,
            ..RouteConfig::default()
        },
        RouteConfig {
            max_vertices: 1,
            ..RouteConfig::default()
        },
    ] {
        assert!(matches!(
            route(endpoints(&nodes), &config),
            Err(RouteFailure::WorkLimit)
        ));
    }
    let mut input = endpoints(&nodes);
    input.source.obstacle = 9;
    assert!(matches!(
        route(input, &RouteConfig::default()),
        Err(RouteFailure::InvalidGeometry)
    ));
    nodes[1].max.x = f32::INFINITY;
    assert!(matches!(
        route(endpoints(&nodes), &RouteConfig::default()),
        Err(RouteFailure::InvalidGeometry)
    ));
}

#[test]
fn enclosed_escape_reports_no_corridor() {
    let nodes = [
        rect(0.0, 0.0, 40.0, 80.0),
        rect(500.0, 0.0, 40.0, 80.0),
        rect(-100.0, -100.0, 300.0, 20.0),
        rect(-100.0, 150.0, 300.0, 20.0),
        rect(-100.0, -100.0, 20.0, 270.0),
        rect(180.0, -100.0, 20.0, 270.0),
    ];
    assert!(matches!(
        route(endpoints(&nodes), &RouteConfig::default()),
        Err(RouteFailure::NoCorridor)
    ));
}

#[test]
fn search_handles_equal_coordinates_and_monotonic_failure_with_a_valid_fallback() {
    let config = RouteConfig::default();
    let obstacles = [
        rect(10.0, -20.0, 20.0, 20.0),
        rect(10.0, 10.0, 20.0, 20.0),
        rect(20.0, -20.0, 10.0, 50.0),
    ];
    let start = Pos2::new(15.0, 5.0);
    let end = Pos2::new(50.0, 5.0);
    let mut budget = WorkBudget::new(config.max_work);
    let channels = Channels::new(start, end, &obstacles, &config, &mut budget).unwrap();
    assert_eq!(
        channels.find(
            start,
            end,
            &obstacles,
            &config,
            false,
            &mut WorkBudget::new(0)
        ),
        Err(RouteFailure::WorkLimit)
    );
    assert_eq!(
        channels.find(start, end, &obstacles, &config, true, &mut budget),
        Err(RouteFailure::NoCorridor)
    );
    let path = channels
        .find(start, end, &obstacles, &config, false, &mut budget)
        .unwrap();
    assert!(path.iter().any(|point| point.x < start.x));
    let equal = Channels::new(start, start, &obstacles, &config, &mut budget).unwrap();
    assert_eq!(
        equal
            .find(start, start, &obstacles, &config, false, &mut budget)
            .unwrap(),
        vec![start]
    );
    let vertical_end = Pos2::new(start.x, 7.0);
    let equal_x = Channels::new(start, vertical_end, &obstacles, &config, &mut budget).unwrap();
    assert_eq!(
        equal_x
            .find(start, vertical_end, &obstacles, &config, false, &mut budget)
            .unwrap(),
        vec![start, vertical_end]
    );
}

#[test]
fn collision_checks_reject_corner_contact_and_diagonal_shortcuts() {
    let obstacles = [rect(10.0, 10.0, 20.0, 20.0)];
    let mut budget = WorkBudget::new(100);
    assert!(
        clear(
            [Pos2::ZERO, Pos2::new(10.0, 0.0)],
            &obstacles,
            None,
            &mut budget
        )
        .unwrap()
    );
    assert!(
        !clear(
            [Pos2::new(0.0, 10.0), Pos2::new(10.0, 10.0)],
            &obstacles,
            None,
            &mut budget
        )
        .unwrap()
    );
    assert!(!clear([Pos2::new(10.0, 10.0); 2], &obstacles, None, &mut budget).unwrap());
    assert_eq!(
        clear(
            [Pos2::ZERO, Pos2::new(40.0, 40.0)],
            &obstacles,
            None,
            &mut budget
        ),
        Err(RouteFailure::InvalidGeometry)
    );
}

#[test]
fn obstacle_iteration_order_does_not_change_cold_routing() {
    let mut nodes = vec![
        rect(0.0, 0.0, 40.0, 80.0),
        rect(600.0, 0.0, 40.0, 80.0),
        rect(150.0, 0.0, 50.0, 90.0),
        rect(350.0, -20.0, 50.0, 110.0),
    ];
    let expected = checked_points(endpoints(&nodes));
    nodes.swap(2, 3);
    assert_eq!(checked_points(endpoints(&nodes)), expected);
}
