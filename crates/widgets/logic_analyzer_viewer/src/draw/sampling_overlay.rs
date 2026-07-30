use egui::{Color32, Painter, Pos2, Shape, Stroke};

use crate::types::{AnalyzerLayout, RowKey};
use crate::viewer::LogicAnalyzerViewer;

const MARKER_SPACING_PX: f64 = 6.0;

impl LogicAnalyzerViewer {
    pub(crate) fn draw_sampling_overlay(&self, painter: &Painter, layout: AnalyzerLayout) {
        let Some(overlay) = &self.sampling_overlay else {
            return;
        };
        let Some(clock_row) = self
            .row_order
            .iter()
            .position(|row| row == &RowKey::Channel(overlay.clock_channel))
        else {
            return;
        };

        if layout.wave_rect.width() <= 0.0 {
            return;
        }
        let visible_end_us = self.visible_start_us + self.visible_span_us;
        let start_ns = us_to_ns(self.visible_start_us);
        let end_ns = us_to_ns(visible_end_us);
        let minimum_spacing_ns = minimum_marker_spacing_ns(
            start_ns,
            end_ns,
            layout.wave_rect.width(),
            MARKER_SPACING_PX,
        );
        let Some(points) = overlay.points.points_in_range_with_minimum_spacing(
            start_ns,
            end_ns,
            minimum_spacing_ns,
        ) else {
            return;
        };
        let edges = points
            .iter()
            .map(|point| (point.time_ns as f64 / 1_000.0, point.clock_high))
            .collect::<Vec<_>>();
        if edges.is_empty() {
            return;
        }
        let clip = painter.with_clip_rect(layout.wave_rect);
        let clock_top = self.row_top(layout.wave_rect.top(), clock_row, layout.row_height);
        let marker_color = Color32::from_rgb(0, 220, 95);
        for &(time_us, rising) in &edges {
            let x = self.time_to_x(layout.wave_rect, time_us);
            draw_clock_arrow(&clip, x, clock_top, layout.row_height, rising, marker_color);
        }

        for (value_index, &channel_index) in overlay.sampled_channels.iter().enumerate() {
            let Some(row) = self
                .row_order
                .iter()
                .position(|key| key == &RowKey::Channel(channel_index))
            else {
                continue;
            };
            let row_top = self.row_top(layout.wave_rect.top(), row, layout.row_height);
            let high_y = row_top + layout.row_height * 0.28;
            let low_y = row_top + layout.row_height * 0.72;
            for point in &points {
                let Some(&value) = point.values.get(value_index) else {
                    continue;
                };
                let time_us = point.time_ns as f64 / 1_000.0;
                let center = Pos2::new(
                    self.time_to_x(layout.wave_rect, time_us),
                    if value { high_y } else { low_y },
                );
                clip.circle_filled(center, 3.4, marker_color);
                clip.circle_stroke(center, 3.4, Stroke::new(0.8, Color32::from_rgb(12, 40, 24)));
            }
        }
    }
}

fn us_to_ns(time_us: f64) -> u64 {
    (time_us * 1_000.0).round().clamp(0.0, u64::MAX as f64) as u64
}

fn minimum_marker_spacing_ns(
    visible_start_ns: u64,
    visible_end_ns: u64,
    width: f32,
    spacing_px: f64,
) -> u64 {
    if width <= 0.0 || visible_start_ns >= visible_end_ns {
        return u64::MAX;
    }
    let visible_span_ns = visible_end_ns - visible_start_ns;
    (visible_span_ns as f64 * spacing_px / f64::from(width)).ceil() as u64
}

fn draw_clock_arrow(
    painter: &Painter,
    x: f32,
    row_top: f32,
    row_height: f32,
    rising: bool,
    color: Color32,
) {
    let high_y = row_top + row_height * 0.28;
    let low_y = row_top + row_height * 0.72;
    let (tip_y, base_y, stem_end) = if rising {
        (high_y - 2.0, high_y + 4.5, high_y + 8.0)
    } else {
        (low_y + 2.0, low_y - 4.5, low_y - 8.0)
    };
    painter.line_segment(
        [Pos2::new(x, base_y), Pos2::new(x, stem_end)],
        Stroke::new(1.2, color),
    );
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(x, tip_y),
            Pos2::new(x - 4.0, base_y),
            Pos2::new(x + 4.0, base_y),
        ],
        color,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_spacing_converts_to_a_conservative_time_distance() {
        assert_eq!(minimum_marker_spacing_ns(0, 100_000, 100.0, 6.0), 6_000);
        assert_eq!(minimum_marker_spacing_ns(0, 100, 64.0, 6.0), 10);
    }
}
