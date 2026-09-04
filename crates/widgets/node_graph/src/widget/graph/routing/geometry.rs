use egui::{Pos2, Rect, Vec2};

/// Exact geometry; disconnected sections remain disconnected during hit testing.
#[derive(Clone, Debug)]
pub(crate) enum PathSegment {
    Line([Pos2; 2]),
    Cubic([Pos2; 4]),
}

/// A layout-space path and its interaction approximation, built together once.
pub(crate) struct WirePath {
    segments: Vec<PathSegment>,
    lines: Vec<[Pos2; 2]>,
    bounds: Rect,
}

impl WirePath {
    pub(crate) fn new(segments: Vec<PathSegment>, tolerance: f32) -> Self {
        assert!(tolerance.is_finite() && tolerance > 0.0);
        let mut lines = Vec::new();
        let mut bounds = Rect::NOTHING;
        for segment in &segments {
            match segment {
                PathSegment::Line(points) => {
                    for point in points {
                        bounds.extend_with(*point);
                    }
                    lines.push(*points);
                }
                PathSegment::Cubic(points) => {
                    for point in points {
                        bounds.extend_with(*point);
                    }
                    flatten(*points, tolerance, 0, &mut lines);
                }
            }
        }
        Self {
            segments,
            lines,
            bounds,
        }
    }

    /// Preserve the existing output-first handles, including their screen-space minimum.
    pub(crate) fn legacy(from: Pos2, to: Pos2, zoom: f32) -> Self {
        let dx = (to.x - from.x).abs().max(50.0 / zoom) * 0.5;
        Self::new(
            vec![PathSegment::Cubic([
                from,
                from + Vec2::new(dx, 0.0),
                to - Vec2::new(dx, 0.0),
                to,
            ])],
            0.5 / zoom,
        )
    }

    pub(crate) fn segments(&self) -> &[PathSegment] {
        &self.segments
    }

    /// Conservative control-hull bounds, including every path section.
    pub(crate) fn bounds(&self) -> Rect {
        self.bounds
    }

    pub(crate) fn distance(&self, point: Pos2) -> f32 {
        self.lines
            .iter()
            .map(|line| distance_to_segment(point, *line))
            .fold(f32::INFINITY, f32::min)
    }

    pub(crate) fn intersects_segment(&self, segment: [Pos2; 2]) -> bool {
        self.bounds()
            .intersects(Rect::from_two_pos(segment[0], segment[1]))
            && self
                .lines
                .iter()
                .any(|line| segments_intersect(*line, segment))
    }

    pub(crate) fn intersects_rect(&self, rect: Rect) -> bool {
        if !self.bounds.intersects(rect) {
            return false;
        }
        let corners = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
        self.lines.iter().any(|line| {
            rect.contains(line[0])
                || rect.contains(line[1])
                || (0..4).any(|i| segments_intersect(*line, [corners[i], corners[(i + 1) % 4]]))
        })
    }
}

fn distance_to_segment(point: Pos2, [a, b]: [Pos2; 2]) -> f32 {
    let delta = b - a;
    if delta.length_sq() == 0.0 {
        return point.distance(a);
    }
    let t = ((point - a).dot(delta) / delta.length_sq()).clamp(0.0, 1.0);
    point.distance(a + t * delta)
}

fn flatten(points: [Pos2; 4], tolerance: f32, depth: u8, lines: &mut Vec<[Pos2; 2]>) {
    // Bounding control-point distance to the finite chord also catches collinear
    // reversals (a distance-to-infinite-line test incorrectly flattens those loops).
    let chord = [points[0], points[3]];
    // Guard pathological/non-finite input as well as floating-point subdivision limits.
    // This interaction approximation is not an obstacle-collision safety proof.
    if depth == 16
        || !points.iter().all(|point| point.is_finite())
        || distance_to_segment(points[1], chord).max(distance_to_segment(points[2], chord))
            <= tolerance
    {
        lines.push(chord);
        return;
    }
    let a = points[0].lerp(points[1], 0.5);
    let b = points[1].lerp(points[2], 0.5);
    let c = points[2].lerp(points[3], 0.5);
    let d = a.lerp(b, 0.5);
    let e = b.lerp(c, 0.5);
    let midpoint = d.lerp(e, 0.5);
    // At floating-point resolution a subdivision can no longer make progress.
    if [points[0], a, d, midpoint] == points || [midpoint, e, c, points[3]] == points {
        lines.push(chord);
        return;
    }
    flatten([points[0], a, d, midpoint], tolerance, depth + 1, lines);
    flatten([midpoint, e, c, points[3]], tolerance, depth + 1, lines);
}

fn segments_intersect([a, b]: [Pos2; 2], [c, d]: [Pos2; 2]) -> bool {
    if !Rect::from_two_pos(a, b).intersects(Rect::from_two_pos(c, d)) {
        return false;
    }
    // f64 intermediates avoid losing the orientation of nearly parallel f32 segments.
    let orientation = |p: Pos2, q: Pos2, r: Pos2| {
        (q.x as f64 - p.x as f64) * (r.y as f64 - p.y as f64)
            - (q.y as f64 - p.y as f64) * (r.x as f64 - p.x as f64)
    };
    let opposite = |x: f64, y: f64| (x <= 0.0 && y >= 0.0) || (x >= 0.0 && y <= 0.0);
    opposite(orientation(a, b, c), orientation(a, b, d))
        && opposite(orientation(c, d, a), orientation(c, d, b))
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn queries_cover_segments_between_samples_and_disconnected_sections() {
        let path = WirePath::new(
            vec![
                PathSegment::Line([Pos2::ZERO, Pos2::new(1000.0, 0.0)]),
                PathSegment::Line([Pos2::new(1000.0, 100.0), Pos2::new(0.0, 100.0)]),
            ],
            0.5,
        );
        assert_eq!(path.distance(Pos2::new(17.0, 3.0)), 3.0);
        assert!(path.intersects_rect(Rect::from_min_max(
            Pos2::new(16.0, -1.0),
            Pos2::new(18.0, 1.0)
        )));
        assert!(path.intersects_segment([Pos2::new(17.0, 99.0), Pos2::new(17.0, 101.0)]));
        assert!(!path.intersects_segment([Pos2::new(999.0, 50.0), Pos2::new(1001.0, 50.0)]));
        assert!(path.bounds().contains(Pos2::new(500.0, 100.0)));
    }

    #[test]
    fn collinear_loops_and_degenerate_segments_are_not_lost() {
        let path = WirePath::new(
            vec![PathSegment::Cubic([
                Pos2::ZERO,
                Pos2::new(100.0, 0.0),
                Pos2::new(-100.0, 0.0),
                Pos2::ZERO,
            ])],
            0.01,
        );
        assert!(path.intersects_segment([Pos2::new(20.0, -1.0), Pos2::new(20.0, 1.0)]));
        assert!(path.intersects_segment([Pos2::new(-20.0, 0.0), Pos2::ZERO]));
        let point = WirePath::new(vec![PathSegment::Line([Pos2::ZERO; 2])], 0.5);
        assert_eq!(point.distance(Pos2::new(3.0, 4.0)), 5.0);
        assert!(!point.intersects_segment([Pos2::new(1.0, 0.0), Pos2::new(2.0, 0.0)]));
    }

    #[test]
    fn flattening_error_and_hit_tolerance_scale_with_zoom() {
        for zoom in [0.2, 1.0, 3.0] {
            let path = WirePath::legacy(Pos2::ZERO, Pos2::new(-80.0, 130.0), zoom);
            let PathSegment::Cubic(p) = path.segments()[0] else {
                panic!("legacy cubic")
            };
            for i in 0..=1000 {
                let t = i as f32 / 1000.0;
                let a = p[0].lerp(p[1], t).lerp(p[1].lerp(p[2], t), t);
                let b = p[1].lerp(p[2], t).lerp(p[2].lerp(p[3], t), t);
                assert!(path.distance(a.lerp(b, t)) * zoom <= 0.501);
            }
        }
    }
}
