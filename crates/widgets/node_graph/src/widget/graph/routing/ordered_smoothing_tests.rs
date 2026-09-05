use egui::{Pos2, Rect, Vec2};

use super::bundle::{BundleMember, route_bundle};
use super::contract::{PortGeometry, PortSide, RouteConfig, RouteInput, WorkBudget};
use super::geometry::{PathSegment, WirePath};
use super::individual::route_with_budget;
use super::ordered_smoothing::smooth_bundle;

fn scene(shared: bool) -> (Vec<Rect>, Vec<BundleMember>) {
    let rect = |x, y, w, h| Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
    let nodes = vec![
        rect(0.0, 0.0, 50.0, 120.0),
        rect(550.0, 0.0, 50.0, 120.0),
        rect(250.0, -5.0, 50.0, 130.0),
    ];
    let members = (0..3)
        .map(|i| BundleMember {
            source: PortGeometry {
                obstacle: 0,
                side: PortSide::Right,
                position: Pos2::new(50.0, if shared { 40.0 } else { 20.0 + 20.0 * i as f32 }),
            },
            target: PortGeometry {
                obstacle: 1,
                side: PortSide::Left,
                position: Pos2::new(550.0, 40.0 + 20.0 * i as f32),
            },
            source_socket: if shared { 0 } else { i },
            target_socket: i,
        })
        .collect();
    (nodes, members)
}

fn checked(nodes: &[Rect], members: &[BundleMember]) -> Vec<WirePath> {
    let config = RouteConfig::default();
    route_bundle(
        nodes,
        members,
        &config,
        &mut WorkBudget::new(config.max_work),
    )
    .unwrap()
}

fn curves(path: &WirePath) -> Vec<[Pos2; 4]> {
    path.segments()
        .iter()
        .filter_map(|s| match s {
            PathSegment::Cubic(p) => Some(*p),
            _ => None,
        })
        .collect()
}

fn evaluate(p: [Pos2; 4], t: f32) -> Pos2 {
    let a = p[0].lerp(p[1], t);
    let b = p[1].lerp(p[2], t);
    let c = p[2].lerp(p[3], t);
    a.lerp(b, t).lerp(b.lerp(c, t), t)
}

fn assert_ordered(paths: &[WirePath], nodes: &[Rect]) {
    let lanes: Vec<_> = paths.iter().map(curves).collect();
    assert!(
        lanes
            .iter()
            .all(|lane| !lane.is_empty() && lane.len() == lanes[0].len())
    );
    for lane in &lanes {
        for curve in lane {
            assert_eq!(curve[0].y, curve[1].y);
            assert_eq!(curve[2].y, curve[3].y);
            assert!(curve[0].x < curve[1].x && curve[2].x < curve[3].x);
            for t in 0..=100 {
                let p = evaluate(*curve, t as f32 / 100.0);
                assert!(
                    nodes
                        .iter()
                        .all(|node| !node.expand2(Vec2::new(20.0, 16.0)).contains(p))
                );
            }
        }
        for pair in lane.windows(2) {
            assert_eq!(pair[0][3], pair[1][0]);
        }
    }
    for pair in lanes.windows(2) {
        for (a, b) in pair[0].iter().zip(&pair[1]) {
            let gap = if a[0].x >= 113.0 && a[3].x <= 487.0 {
                8.0
            } else {
                0.0
            };
            for k in 0..4 {
                assert_eq!(a[k].x, b[k].x);
                assert!(b[k].y as f64 - a[k].y as f64 >= gap);
            }
            for t in 0..=100 {
                let a = evaluate(*a, t as f32 / 100.0);
                let b = evaluate(*b, t as f32 / 100.0);
                assert_eq!(a.x, b.x);
                assert!(a.y <= b.y);
            }
        }
    }
}

#[test]
fn common_x_curves_preserve_order_spacing_and_escape_segments() {
    let config = RouteConfig::default();
    for shared in [false, true] {
        let (nodes, members) = scene(shared);
        let original = checked(&nodes, &members);
        let paths = smooth_bundle(
            &nodes,
            &members,
            checked(&nodes, &members),
            &config,
            1.0,
            &mut WorkBudget::new(config.max_smoothing_work),
        );
        assert_ordered(&paths, &nodes);
        for (path, original) in paths.iter().zip(&original) {
            let PathSegment::Line(first) = path.segments()[0] else {
                panic!("source escape")
            };
            let PathSegment::Line(last) = path.segments().last().unwrap() else {
                panic!("target escape")
            };
            let lane = curves(path);
            assert_eq!(first[1], lane[0][0]);
            assert_eq!(last[0], lane.last().unwrap()[3]);
            assert_eq!(
                format!("{:?}", path.segments().first()),
                format!("{:?}", original.segments().first())
            );
            assert_eq!(
                format!("{:?}", path.segments().last()),
                format!("{:?}", original.segments().last())
            );
        }
    }
}

#[test]
fn checked_bundle_integration_widens_locally_and_preserves_shared_target_fans() {
    let config = RouteConfig::default();
    let (nodes, mut members) = scene(false);
    for member in &mut members {
        member.target.position.y = 60.0;
        member.target_socket = 0;
    }
    let paths = smooth_bundle(
        &nodes,
        &members,
        checked(&nodes, &members),
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_ordered(&paths, &nodes);
    let lanes: Vec<_> = paths.iter().map(curves).collect();
    let first = lanes[0].first().unwrap()[0].x + 32.0;
    let last = lanes[0].last().unwrap()[3].x - 32.0;
    let gaps: Vec<_> = lanes[0]
        .iter()
        .zip(&lanes[1])
        .filter_map(|(a, b)| (a[0].x >= first && a[0].x <= last).then_some(b[0].y - a[0].y))
        .collect();
    assert!(
        gaps.contains(&8.0),
        "preserve minimum-spacing fan boundary: {gaps:?}"
    );
    assert!(
        gaps.iter().any(|gap| *gap >= 12.0),
        "widen the interior: {gaps:?}"
    );
    for pair in lanes.windows(2) {
        assert_eq!(pair[0].last().unwrap()[3], pair[1].last().unwrap()[3]);
    }
}

#[test]
fn quality_exhaustion_restores_the_whole_group() {
    let (nodes, members) = scene(true);
    let config = RouteConfig::default();
    let original: Vec<_> = checked(&nodes, &members)
        .iter()
        .map(|p| format!("{:?}", p.segments()))
        .collect();
    for work in [0, 1, 50, 500, config.max_smoothing_work] {
        let paths = smooth_bundle(
            &nodes,
            &members,
            checked(&nodes, &members),
            &config,
            1.0,
            &mut WorkBudget::new(work),
        );
        if curves(&paths[0]).is_empty() {
            assert_eq!(
                original,
                paths
                    .iter()
                    .map(|p| format!("{:?}", p.segments()))
                    .collect::<Vec<_>>()
            );
        } else {
            assert_ordered(&paths, &nodes);
        }
    }
}

#[test]
fn reversed_endpoint_order_is_not_certified_as_a_smooth_bundle() {
    let (nodes, mut members) = scene(false);
    members[0].target.position.y = 80.0;
    members[2].target.position.y = 40.0;
    let config = RouteConfig::default();
    // Individually checked paths are not necessarily a valid ordered group.
    let paths: Vec<_> = members
        .iter()
        .map(|m| {
            route_with_budget(
                RouteInput {
                    nodes: &nodes,
                    source: m.source,
                    target: m.target,
                },
                &config,
                &mut WorkBudget::new(config.max_work),
            )
            .unwrap()
        })
        .collect();
    let original: Vec<_> = paths
        .iter()
        .map(|p| format!("{:?}", p.segments()))
        .collect();
    let paths = smooth_bundle(
        &nodes,
        &members,
        paths,
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_eq!(
        original,
        paths
            .iter()
            .map(|p| format!("{:?}", p.segments()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn obstacle_in_every_rounding_window_keeps_the_checked_bundle() {
    let (mut nodes, members) = scene(true);
    let config = RouteConfig {
        clearance_x: 0.0,
        clearance_y: 0.0,
        ..RouteConfig::default()
    };
    let paths = route_bundle(
        &nodes,
        &members,
        &config,
        &mut WorkBudget::new(config.max_work),
    )
    .unwrap();
    let vertical = paths[0]
        .segments()
        .iter()
        .find_map(|s| match s {
            PathSegment::Line(p) if p[0].x == p[1].x && p[0].y != p[1].y => Some(*p),
            _ => None,
        })
        .unwrap();
    let obstacle = Rect::from_min_max(
        Pos2::new(
            vertical[0].x + 0.0001,
            vertical[0].y.min(vertical[1].y) + 0.0001,
        ),
        Pos2::new(
            vertical[0].x + 0.0002,
            vertical[0].y.max(vertical[1].y) - 0.0001,
        ),
    );
    // The new body leaves every checked segment clear but blocks the space that
    // interpolation needs beside this vertical run, even at the smallest retry.
    for path in &paths {
        for segment in path.segments() {
            let PathSegment::Line(p) = segment else {
                panic!("checked line")
            };
            assert!(!Rect::from_two_pos(p[0], p[1]).intersects(obstacle));
        }
    }
    nodes.push(obstacle);
    let original: Vec<_> = paths
        .iter()
        .map(|p| format!("{:?}", p.segments()))
        .collect();
    let result = smooth_bundle(
        &nodes,
        &members,
        paths,
        &config,
        1.0,
        &mut WorkBudget::new(config.max_smoothing_work),
    );
    assert_eq!(
        original,
        result
            .iter()
            .map(|p| format!("{:?}", p.segments()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ordered_curve_visual_fixture_and_interactions_are_zoom_stable() {
    let (nodes, members) = scene(true);
    let config = RouteConfig::default();
    let mut original = Vec::new();
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1100' height='1050' viewBox='0 0 1100 1050'><rect width='1100' height='1050' fill='#1c1c1c'/>",
    );
    for (row, zoom) in [0.5, 1.0, 1.7].into_iter().enumerate() {
        let paths = smooth_bundle(
            &nodes,
            &members,
            checked(&nodes, &members),
            &config,
            zoom,
            &mut WorkBudget::new(config.max_smoothing_work),
        );
        assert_ordered(&paths, &nodes);
        let geometry: Vec<_> = paths.iter().map(curves).collect();
        if original.is_empty() {
            original = geometry;
        } else {
            assert_eq!(original, geometry);
        }
        svg.push_str(&format!("<g transform='translate(30,{})'><text fill='white' font-size='16' y='25'>Ordered smooth bundle: {zoom}x</text><g transform='translate(0,100) scale({zoom})'>", row * 350));
        for (path, color) in paths.iter().zip(["#62b6ff", "#80d99b", "#e9bd70"]) {
            for segment in path.segments() {
                match segment {
                    PathSegment::Line([a, b]) => svg.push_str(&format!(
                        "<path d='M {} {} L {} {}' fill='none' stroke='{color}' stroke-width='2'/>",
                        a.x, a.y, b.x, b.y
                    )),
                    PathSegment::Cubic(p) => {
                        for t in 0..=100 {
                            assert!(path.distance(evaluate(*p, t as f32 / 100.0)) * zoom <= 0.501);
                        }
                        svg.push_str(&format!("<path d='M {} {} C {} {},{} {},{} {}' fill='none' stroke='{color}' stroke-width='2'/>", p[0].x,p[0].y,p[1].x,p[1].y,p[2].x,p[2].y,p[3].x,p[3].y));
                    }
                }
            }
        }
        for node in &nodes {
            svg.push_str(&format!(
                "<rect x='{}' y='{}' width='{}' height='{}' fill='#40454d' stroke='#88909b'/>",
                node.min.x,
                node.min.y,
                node.width(),
                node.height()
            ));
        }
        svg.push_str("</g></g>");
    }
    svg.push_str("</svg>");
    println!("{svg}");
}
