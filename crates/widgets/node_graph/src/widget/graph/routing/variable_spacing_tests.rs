use egui::{Pos2, Rect};

use super::contract::WorkBudget;
use super::corridor::cubic_clear;
use super::geometry::{PathSegment, WirePath};
use super::variable_spacing::widen_spacing;

fn lanes() -> Vec<Vec<[Pos2; 4]>> {
    [0.0, 8.0]
        .into_iter()
        .map(|y| {
            (0..5)
                .map(|i| {
                    let x = i as f32 * 20.0;
                    [
                        Pos2::new(x, y),
                        Pos2::new(x + 20.0 / 3.0, y),
                        Pos2::new(x + 40.0 / 3.0, y),
                        Pos2::new(x + 20.0, y),
                    ]
                })
                .collect()
        })
        .collect()
}

fn constriction() -> Vec<Rect> {
    vec![
        Rect::from_min_max(Pos2::new(40.0, -15.0), Pos2::new(60.0, -0.01)),
        Rect::from_min_max(Pos2::new(40.0, 8.01), Pos2::new(60.0, 23.0)),
    ]
}

fn assert_valid(curves: &[Vec<[Pos2; 4]>], obstacles: &[Rect]) {
    for lane in curves {
        for curve in lane {
            assert!(cubic_clear(*curve, obstacles, &mut WorkBudget::new(50_000)).unwrap());
            assert_eq!(curve[0].y, curve[1].y);
            assert_eq!(curve[2].y, curve[3].y);
        }
        for join in lane.windows(2) {
            assert_eq!(join[0][3], join[1][0]);
        }
    }
    for pair in curves.windows(2) {
        for (a, b) in pair[0].iter().zip(&pair[1]) {
            for k in 0..4 {
                assert_eq!(a[k].x, b[k].x);
                assert!(b[k].y as f64 - a[k].y as f64 >= 8.0);
            }
        }
    }
}

#[test]
fn spacing_widens_in_open_sections_and_contracts_through_the_bottleneck() {
    let obstacles = constriction();
    let mut curves = lanes();
    widen_spacing(
        &mut curves,
        &obstacles,
        [0.0, 100.0],
        12.0,
        &mut WorkBudget::new(50_000),
    );
    assert_valid(&curves, &obstacles);
    let gaps: Vec<_> = (0..5)
        .map(|i| curves[1][i][0].y - curves[0][i][0].y)
        .collect();
    assert_eq!(gaps, [8.0, 12.0, 8.0, 8.0, 12.0]);
    for (actual, original) in curves.iter().zip(lanes()) {
        assert_eq!(actual[0][0], original[0][0]);
        assert_eq!(actual[4][3], original[4][3]);
    }

    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='720' height='640'><rect width='100%' height='100%' fill='#202228'/>",
    );
    for (row, zoom) in [0.5, 1.0, 1.7].into_iter().enumerate() {
        svg.push_str(&format!("<g transform='translate(30,{})'><text fill='white' y='-100'>Local bundle spacing: {zoom}x</text><g transform='scale({})'>", 130 + row * 180, zoom * 3.0));
        for rect in &obstacles {
            svg.push_str(&format!(
                "<rect x='{}' y='{}' width='{}' height='{}' fill='#55606b'/>",
                rect.min.x,
                rect.min.y,
                rect.width(),
                rect.height()
            ));
        }
        for lane in &curves {
            let path = WirePath::new(
                lane.iter().copied().map(PathSegment::Cubic).collect(),
                0.5 / zoom,
            );
            for p in lane {
                svg.push_str(&format!("<path d='M {} {} C {} {},{} {},{} {}' stroke='#7bd9ed' stroke-width='0.6' fill='none'/>", p[0].x,p[0].y,p[1].x,p[1].y,p[2].x,p[2].y,p[3].x,p[3].y));
                for i in 0..=100 {
                    let t = i as f32 / 100.0;
                    let a = p[0].lerp(p[1], t).lerp(p[1].lerp(p[2], t), t);
                    let b = p[1].lerp(p[2], t).lerp(p[2].lerp(p[3], t), t);
                    assert!(path.distance(a.lerp(b, t)) * zoom <= 0.501);
                }
            }
        }
        svg.push_str("</g></g>");
    }
    svg.push_str("</svg>");
    println!("{svg}");
}

#[test]
fn asymmetric_capacity_can_shift_the_centerline() {
    let obstacles = vec![Rect::from_min_max(
        Pos2::new(5.0, -15.0),
        Pos2::new(95.0, -0.01),
    )];
    let mut curves = lanes();
    widen_spacing(
        &mut curves,
        &obstacles,
        [0.0, 100.0],
        12.0,
        &mut WorkBudget::new(50_000),
    );
    assert_valid(&curves, &obstacles);
    assert_eq!(curves[0][2][0].y, 0.0);
    assert_eq!(curves[1][2][0].y, 12.0);
}

#[test]
fn every_budget_cutoff_retains_a_complete_certified_bundle() {
    let obstacles = constriction();
    for work in [0, 1, 20, 50, 100, 500, 1000, 5000, 50_000] {
        let mut curves = lanes();
        widen_spacing(
            &mut curves,
            &obstacles,
            [0.0, 100.0],
            12.0,
            &mut WorkBudget::new(work),
        );
        assert_valid(&curves, &obstacles);
        if work == 0 {
            assert_eq!(curves, lanes());
        }
    }
}
