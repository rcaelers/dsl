use egui::{Pos2, Rect, Vec2};

use super::bundle::{BundleMember, route_bundle};
use super::bundle_corridor::route_lanes;
use super::contract::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget};
use super::geometry::{PathSegment, WirePath};
use super::individual::route_with_budget;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
}

fn scene(shared: bool) -> (Vec<Rect>, Vec<BundleMember>) {
    let nodes = vec![rect(0.0, 0.0, 50.0, 120.0), rect(550.0, 0.0, 50.0, 120.0)];
    let members = (0..3)
        .map(|i| BundleMember {
            source: PortGeometry {
                obstacle: 0,
                side: PortSide::Right,
                position: Pos2::new(50.0, if shared { 40.0 } else { 20.0 + i as f32 * 20.0 }),
            },
            target: PortGeometry {
                obstacle: 1,
                side: PortSide::Left,
                position: Pos2::new(550.0, 40.0 + i as f32 * 20.0),
            },
            source_socket: if shared { 0 } else { i },
            target_socket: i,
        })
        .collect();
    (nodes, members)
}

fn route(nodes: &[Rect], members: &[BundleMember]) -> Result<Vec<WirePath>, RouteFailure> {
    let config = RouteConfig::default();
    route_bundle(
        nodes,
        members,
        &config,
        &mut WorkBudget::new(config.max_work),
    )
}

fn lines(path: &WirePath) -> Vec<[Pos2; 2]> {
    path.segments()
        .iter()
        .map(|s| match s {
            PathSegment::Line(p) => *p,
            _ => panic!("rectilinear bundle"),
        })
        .collect()
}

fn check_obstacles(nodes: &[Rect], members: &[BundleMember], paths: &[WirePath]) {
    assert_eq!(members.len(), paths.len());
    for (member, path) in members.iter().zip(paths) {
        let lines = lines(path);
        assert_eq!(lines[0][0], member.source.position);
        assert_eq!(lines.last().unwrap()[1], member.target.position);
        for pair in lines.windows(2) {
            assert_eq!(pair[0][1], pair[1][0]);
        }
        for (i, line) in lines.iter().enumerate() {
            assert!(line[0].is_finite() && line[1].is_finite());
            assert!(line[0].x <= line[1].x);
            assert!(line[0].x == line[1].x || line[0].y == line[1].y);
            for (node_index, node) in nodes.iter().enumerate() {
                if (i == 0 && node_index == member.source.obstacle)
                    || (i == lines.len() - 1 && node_index == member.target.obstacle)
                {
                    continue;
                }
                assert!(
                    !Rect::from_two_pos(line[0], line[1])
                        .intersects(node.expand2(Vec2::new(20.0, 16.0)))
                );
            }
        }
    }
}

fn check_lanes(paths: &[WirePath]) {
    let trunks: Vec<_> = paths
        .iter()
        .map(|p| {
            lines(p)
                .into_iter()
                .filter(|p| p[0].y == p[1].y)
                .max_by(|a, b| (a[1].x - a[0].x).total_cmp(&(b[1].x - b[0].x)))
                .unwrap()
        })
        .collect();
    for pair in trunks.windows(2) {
        assert_eq!(pair[0][0].x, pair[1][0].x);
        assert_eq!(pair[0][1].x, pair[1][1].x);
        assert!(pair[1][0].y - pair[0][0].y >= 8.0);
    }
}

#[test]
fn shared_band_has_spaced_lanes_and_nonintersecting_fans() {
    let (mut nodes, members) = scene(false);
    for obstructed in [false, true] {
        if obstructed {
            nodes.push(rect(250.0, -5.0, 50.0, 130.0));
        }
        let paths = route(&nodes, &members).unwrap();
        check_obstacles(&nodes, &members, &paths);
        check_lanes(&paths);
        for (i, a) in paths.iter().enumerate() {
            for b in paths.iter().skip(i + 1) {
                for a in lines(a) {
                    for b in lines(b) {
                        assert!(
                            !Rect::from_two_pos(a[0], a[1])
                                .intersects(Rect::from_two_pos(b[0], b[1]))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn remote_obstacles_do_not_change_clear_bundle_geometry_and_all_bodies_remain_checked() {
    let (mut nodes, members) = scene(false);
    let baseline = route(&nodes, &members).unwrap();
    for i in 0..500 {
        nodes.push(rect(250.0, 1000.0 + 150.0 * i as f32, 50.0, 80.0));
    }
    let paths = route(&nodes, &members).unwrap();
    for (expected, actual) in baseline.iter().zip(&paths) {
        assert_eq!(
            format!("{:?}", expected.segments()),
            format!("{:?}", actual.segments())
        );
    }
    check_obstacles(&nodes, &members, &paths);
    // A newly relevant body cannot be lost among filtered remote rectangles.
    nodes.push(rect(250.0, -5.0, 50.0, 130.0));
    let detour = route(&nodes, &members).unwrap();
    assert_ne!(
        format!("{:?}", paths[0].segments()),
        format!("{:?}", detour[0].segments())
    );
    check_obstacles(&nodes, &members, &detour);
    check_lanes(&detour);
}

#[test]
fn shared_output_overlaps_only_its_horizontal_fan_prefix() {
    let (nodes, members) = scene(true);
    let paths = route(&nodes, &members).unwrap();
    check_obstacles(&nodes, &members, &paths);
    check_lanes(&paths);
    let mut overlaps = 0;
    for (i, a) in paths.iter().enumerate() {
        for b in paths.iter().skip(i + 1) {
            for a in lines(a) {
                for b in lines(b) {
                    let intersection =
                        Rect::from_two_pos(a[0], a[1]).intersect(Rect::from_two_pos(b[0], b[1]));
                    if !intersection.is_negative() {
                        overlaps += 1;
                        assert_eq!(intersection.min.y, 40.0);
                        assert_eq!(intersection.max.y, 40.0);
                        assert!(intersection.min.x >= 50.0 && intersection.max.x <= 112.001);
                    }
                }
            }
        }
    }
    assert!(overlaps > 0);
}

#[test]
fn coincident_distinct_ports_are_not_treated_as_a_shared_output() {
    let (nodes, mut members) = scene(true);
    for (i, member) in members.iter_mut().enumerate() {
        member.source_socket = i;
    }
    assert!(matches!(
        route(&nodes, &members),
        Err(RouteFailure::NoCorridor)
    ));
}

#[test]
fn narrow_band_and_blocked_connecting_opening_reject_bundle_but_allow_individuals() {
    for opening_blocked in [false, true] {
        let (mut nodes, mut members) = scene(false);
        if opening_blocked {
            nodes.push(rect(110.0, -500.0, 1.0, 1000.0));
        } else {
            nodes.extend([
                rect(-1000.0, -1000.0, 3000.0, 1030.0),
                rect(-1000.0, 70.0, 3000.0, 1000.0),
            ]);
            for (i, member) in members.iter_mut().enumerate() {
                member.source.position.y = 48.0 + i as f32 * 2.0;
                member.target.position.y = member.source.position.y;
            }
        }
        assert!(matches!(
            route(&nodes, &members),
            Err(RouteFailure::NoCorridor)
        ));
        let config = RouteConfig::default();
        for member in &members {
            assert!(
                route_with_budget(
                    RouteInput {
                        nodes: &nodes,
                        source: member.source,
                        target: member.target
                    },
                    &config,
                    &mut WorkBudget::new(config.max_work)
                )
                .is_ok()
            );
        }
    }
}

#[test]
fn bundle_rejects_inversions_invalid_geometry_and_exhausted_work() {
    let (nodes, mut members) = scene(false);
    let config = RouteConfig::default();
    assert!(matches!(
        route_bundle(&nodes, &members, &config, &mut WorkBudget::new(0)),
        Err(RouteFailure::WorkLimit)
    ));
    members.swap(0, 2);
    assert!(matches!(
        route(&nodes, &members),
        Err(RouteFailure::NoCorridor)
    ));
    members.swap(0, 2);
    members[0].source.position.x = f32::NAN;
    assert!(matches!(
        route(&nodes, &members),
        Err(RouteFailure::InvalidGeometry)
    ));
}

#[test]
fn obstacle_iteration_order_does_not_change_band_selection() {
    let (mut nodes, members) = scene(false);
    nodes.extend([
        rect(220.0, 0.0, 30.0, 100.0),
        rect(350.0, -20.0, 40.0, 130.0),
    ]);
    let before: Vec<_> = route(&nodes, &members).unwrap().iter().map(lines).collect();
    nodes.swap(2, 3);
    assert_eq!(
        before,
        route(&nodes, &members)
            .unwrap()
            .iter()
            .map(lines)
            .collect::<Vec<_>>()
    );
}

fn multi_turn_scene() -> (Vec<Rect>, Vec<BundleMember>) {
    let (mut nodes, members) = scene(false);
    // Ceiling/floor seal the outside; alternating attached obstacles require a
    // low band followed by a high band. No single horizontal band can pass both.
    nodes.extend([
        rect(-1000.0, -1000.0, 3000.0, 920.0),
        rect(-1000.0, 180.0, 3000.0, 1000.0),
        rect(200.0, -100.0, 50.0, 170.0),
        rect(350.0, 80.0, 50.0, 120.0),
    ]);
    (nodes, members)
}

fn check_interior_order(paths: &[WirePath], left: f32, right: f32) {
    let all_lines: Vec<_> = paths.iter().map(lines).collect();
    let mut xs = vec![left, right];
    xs.extend(
        all_lines
            .iter()
            .flatten()
            .flatten()
            .map(|p| p.x)
            .filter(|&x| left <= x && x <= right),
    );
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let mids: Vec<_> = xs
        .windows(2)
        .map(|p| (p[0] as f64 * 0.5 + p[1] as f64 * 0.5) as f32)
        .collect();
    xs.extend(mids);
    // Rectilinear paths have constant Y between these event coordinates. Testing
    // full vertical ranges at events also covers the entire turn, not samples on it.
    for x in xs {
        let ranges: Vec<_> = all_lines
            .iter()
            .map(|path| {
                let mut ys = path
                    .iter()
                    .filter(|p| p[0].x <= x && x <= p[1].x)
                    .flat_map(|p| [p[0].y, p[1].y]);
                let first = ys.next().expect("continuous monotonic lane");
                ys.fold((first, first), |(low, high), y| (low.min(y), high.max(y)))
            })
            .collect();
        for pair in ranges.windows(2) {
            assert!(
                pair[1].0 as f64 - pair[0].1 as f64 >= 8.0,
                "lane spacing at {x}: {ranges:?}"
            );
        }
    }
}

#[test]
fn alternating_obstacles_require_checked_ordered_interior_turns() {
    let (mut nodes, mut members) = multi_turn_scene();
    for shared in [false, true] {
        if shared {
            for member in &mut members {
                member.source.position.y = 40.0;
                member.source_socket = 0;
            }
        }
        let paths = route(&nodes, &members).unwrap();
        check_obstacles(&nodes, &members, &paths);
        check_interior_order(&paths, 113.0, 487.0);
        let y_at = |path: &WirePath, x: f32| {
            lines(path)
                .into_iter()
                .find(|p| p[0].x <= x && x <= p[1].x && p[0].y == p[1].y)
                .unwrap()[0]
                .y
        };
        assert!(y_at(&paths[0], 225.0) > 86.0);
        assert!(y_at(&paths[2], 375.0) < 64.0);
        let before: Vec<_> = paths.iter().map(lines).collect();
        nodes[2..].reverse();
        assert_eq!(
            before,
            route(&nodes, &members)
                .unwrap()
                .iter()
                .map(lines)
                .collect::<Vec<_>>()
        );
        nodes[2..].reverse();
    }
}

#[test]
fn multi_turn_bundle_visual_fixture_preserves_spacing_at_each_zoom() {
    let (nodes, members) = multi_turn_scene();
    let paths = route(&nodes, &members).unwrap();
    check_obstacles(&nodes, &members, &paths);
    check_interior_order(&paths, 113.0, 487.0);
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1100' height='1560' viewBox='0 0 1100 1560'><rect width='1100' height='1560' fill='#1c1c1c'/>",
    );
    for (row, zoom) in [0.5, 1.0, 1.7].into_iter().enumerate() {
        svg.push_str(&format!("<g transform='translate(30,{})'><text fill='white' font-size='16' y='25'>Checked multi-turn bundle: {zoom}x</text><g transform='translate(0,180) scale({zoom})'>", row * 520));
        for (path, color) in paths.iter().zip(["#62b6ff", "#80d99b", "#e9bd70"]) {
            for [a, b] in lines(path) {
                svg.push_str(&format!(
                    "<path d='M {} {} L {} {}' fill='none' stroke='{color}' stroke-width='2'/>",
                    a.x, a.y, b.x, b.y
                ));
            }
        }
        let clip = rect(0.0, -90.0, 600.0, 280.0);
        for node in &nodes {
            let node = node.intersect(clip);
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

#[test]
fn narrow_interior_opening_rejects_the_envelope_not_just_its_reference_path() {
    let (mut nodes, members) = multi_turn_scene();
    // Close the transition between the two attached obstacles: their expanded
    // bodies leave an 8-unit X gap, enough for an individual but not a bundle.
    nodes[5] = rect(298.0, 80.0, 102.0, 120.0);
    assert!(matches!(
        route(&nodes, &members),
        Err(RouteFailure::NoCorridor)
    ));
    let config = RouteConfig::default();
    for member in &members {
        assert!(
            route_with_budget(
                RouteInput {
                    nodes: &nodes,
                    source: member.source,
                    target: member.target
                },
                &config,
                &mut WorkBudget::new(config.max_work)
            )
            .is_ok()
        );
    }
}

#[test]
fn shared_search_preserves_work_limits_and_rejects_invalid_lane_counts() {
    let config = RouteConfig {
        max_vertices: 1,
        ..RouteConfig::default()
    };
    let entry = Pos2::new(100.0, 0.0);
    let exit = Pos2::new(500.0, 20.0);
    assert!(matches!(
        route_lanes(
            entry,
            exit,
            &[],
            3,
            &config,
            &mut WorkBudget::new(config.max_work)
        ),
        Err(RouteFailure::WorkLimit)
    ));
    assert!(matches!(
        route_lanes(
            entry,
            exit,
            &[],
            0,
            &config,
            &mut WorkBudget::new(config.max_work)
        ),
        Err(RouteFailure::InvalidGeometry)
    ));
}

/// Portable visual fixture: emit SVG with `--nocapture` after verifying the geometry.
#[test]
fn bundled_fan_out_visual_fixture_has_checked_spaced_lanes_at_each_zoom() {
    let (mut nodes, members) = scene(true);
    nodes.push(rect(250.0, -5.0, 50.0, 130.0));
    let paths = route(&nodes, &members).unwrap();
    check_obstacles(&nodes, &members, &paths);
    check_lanes(&paths);
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1100' height='1050' viewBox='0 0 1100 1050'><rect width='1100' height='1050' fill='#1c1c1c'/>",
    );
    for (row, zoom) in [0.5, 1.0, 1.7].into_iter().enumerate() {
        svg.push_str(&format!("<g transform='translate(30,{})'><text fill='white' font-size='16' y='25'>Checked shared-output bundle: {zoom}x</text><g transform='translate(0,100) scale({zoom})'>", row * 350));
        for (path, color) in paths.iter().zip(["#62b6ff", "#80d99b", "#e9bd70"]) {
            for [a, b] in lines(path) {
                svg.push_str(&format!(
                    "<path d='M {} {} L {} {}' fill='none' stroke='{color}' stroke-width='2'/>",
                    a.x, a.y, b.x, b.y
                ));
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
