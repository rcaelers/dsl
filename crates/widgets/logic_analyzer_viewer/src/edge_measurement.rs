use egui::{PointerButton, Response, Ui};

use crate::cursor::nearest_transition_time;
use crate::sampling::{sample_to_us, us_to_sample};
use crate::types::{AnalyzerLayout, EdgeDeltaMeasurement, RowKey};
use crate::viewer::LogicAnalyzerViewer;

const EDGE_HIT_DISTANCE_PX: f32 = 6.0;

impl LogicAnalyzerViewer {
    /// Starts or stops the click-to-measure edge delta interaction.
    ///
    /// A first primary click must land on a real transition. While active,
    /// the nearest transition on that same row follows the pointer; Escape or
    /// a subsequent primary click stops the measurement.
    pub(crate) fn handle_edge_measurement_input(
        &mut self,
        ui: &Ui,
        response: &Response,
        layout: AnalyzerLayout,
        button: PointerButton,
    ) -> bool {
        if self.edge_delta_measurement.is_some() {
            let cancelled = ui
                .ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if cancelled || response.clicked_by(button) {
                self.edge_delta_measurement = None;
            }
            return true;
        }

        if !self.measurements_enabled || !response.clicked_by(button) {
            return false;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return false;
        };
        let wave_rect = layout.wave_rect;
        if !wave_rect.contains(pointer) || wave_rect.width() <= 1.0 {
            return false;
        }
        let Some(channel_row) = self.row_at_vertical(wave_rect.top(), pointer.y, layout.row_height)
        else {
            return false;
        };
        let time_us = self.x_to_time(wave_rect, pointer.x);
        let Some(edge_us) = self.nearest_edge_at_row(channel_row, time_us) else {
            return false;
        };
        let edge_x = self.time_to_x_unclamped(wave_rect, edge_us);
        if (pointer.x - edge_x).abs() > EDGE_HIT_DISTANCE_PX {
            return false;
        }
        let row_top = self.row_top(wave_rect.top(), channel_row, layout.row_height);
        self.edge_delta_measurement = Some(EdgeDeltaMeasurement {
            channel_row,
            start_us: edge_us,
            end_us: edge_us,
            end_y: row_top
                + self.display_row_height(
                    self.row_order
                        .get(channel_row)
                        .expect("selected row exists"),
                    layout.row_height,
                ) * 0.5,
        });
        true
    }

    /// Updates the selected endpoint from the lane under the pointer. It
    /// snaps to a nearby exact transition, but otherwise remains free at the
    /// pointer's time and vertical position.
    pub(crate) fn update_edge_measurement(
        &mut self,
        layout: AnalyzerLayout,
        pointer: Option<egui::Pos2>,
    ) {
        if self.edge_delta_measurement.is_none() {
            return;
        }
        let Some(pointer) = pointer.filter(|pointer| layout.wave_rect.contains(*pointer)) else {
            return;
        };
        let raw_time_us = self.x_to_time(layout.wave_rect, pointer.x);
        let target_row = self.row_at_vertical(layout.wave_rect.top(), pointer.y, layout.row_height);
        let snapped = target_row
            .filter(|_| self.snapping_enabled)
            .and_then(|row| {
                self.nearest_edge_at_row(row, raw_time_us)
                    .and_then(|edge_us| {
                        let edge_x = self.time_to_x_unclamped(layout.wave_rect, edge_us);
                        ((pointer.x - edge_x).abs() <= EDGE_HIT_DISTANCE_PX)
                            .then_some((row, edge_us))
                    })
            });
        let (end_us, end_y) = if let Some((row, edge_us)) = snapped {
            let row_top = self.row_top(layout.wave_rect.top(), row, layout.row_height);
            let row_height = self.display_row_height(
                self.row_order.get(row).expect("target row exists"),
                layout.row_height,
            );
            (edge_us, row_top + row_height * 0.5)
        } else {
            (raw_time_us, pointer.y)
        };
        if let Some(measurement) = &mut self.edge_delta_measurement {
            measurement.end_us = end_us;
            measurement.end_y = end_y;
        }
    }

    fn nearest_edge_at_row(&mut self, row: usize, time_us: f64) -> Option<f64> {
        let (channel_index, indexed, visible_edge) = {
            let channel = self.channel_at_row(row)?;
            (
                channel.index,
                matches!(self.row_order.get(row), Some(RowKey::Channel(_)))
                    && self.has_index_sampler(),
                nearest_transition_time(&channel.transitions, time_us),
            )
        };
        if !indexed {
            return visible_edge;
        }

        let samplerate_hz = self.capture_info.as_ref()?.header.samplerate_hz;
        let sample = us_to_sample(time_us, samplerate_hz);
        let before = self
            .prev_transition_at_or_before(channel_index, sample)
            .map(|(sample, _)| sample_to_us(sample, samplerate_hz));
        let after = self
            .next_transition_after(channel_index, sample)
            .map(|(sample, _)| sample_to_us(sample, samplerate_hz));
        match (before, after) {
            (Some(before), Some(after)) => Some(if time_us - before <= after - time_us {
                before
            } else {
                after
            }),
            (before, after) => before.or(after),
        }
    }
}

#[cfg(test)]
mod edge_measurement_tests {
    use egui::{Pos2, Rect};

    use crate::cursor::nearest_transition_time;
    use crate::types::{AnalyzerLayout, EdgeDeltaMeasurement, Transition};
    use crate::{ChannelSignal, LogicAnalyzerViewer};

    #[test]
    fn nearest_edge_prefers_the_closer_transition() {
        let transitions = [
            Transition {
                time_us: 10.0,
                value: true,
            },
            Transition {
                time_us: 30.0,
                value: false,
            },
        ];

        assert_eq!(nearest_transition_time(&transitions, 19.0), Some(10.0));
        assert_eq!(nearest_transition_time(&transitions, 21.0), Some(30.0));
    }

    #[test]
    fn endpoint_selection_uses_the_target_lane() {
        let mut viewer = LogicAnalyzerViewer::new();
        viewer.set_channels(vec![
            ChannelSignal {
                index: 0,
                name: "Clock".into(),
                initial: false,
                transitions: vec![(10.0, true), (30.0, false)],
            },
            ChannelSignal {
                index: 1,
                name: "Data".into(),
                initial: false,
                transitions: vec![(12.0, true), (40.0, false)],
            },
        ]);

        assert_eq!(viewer.nearest_edge_at_row(0, 29.0), Some(30.0));
        assert_eq!(viewer.nearest_edge_at_row(1, 29.0), Some(40.0));
    }

    /// Disabling measurement drops what is on screen, and disabling snapping
    /// leaves a still-running measurement's endpoint at the pointer.
    #[test]
    fn toggles_drop_measurements_and_free_the_endpoint() {
        let mut viewer = LogicAnalyzerViewer::new();
        viewer.set_channels(vec![ChannelSignal {
            index: 0,
            name: "Clock".into(),
            initial: false,
            transitions: vec![(10.0, true), (40.0, false)],
        }]);
        viewer.visible_start_us = 0.0;
        viewer.visible_span_us = 100.0;
        let layout = AnalyzerLayout {
            ruler_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 20.0)),
            labels_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(20.0, 100.0)),
            wave_rect: Rect::from_min_max(Pos2::new(0.0, 20.0), Pos2::new(100.0, 100.0)),
            row_height: 30.0,
            trigger_width: 0.0,
            name_col_width: 0.0,
            badge_width: 0.0,
        };
        let measurement = EdgeDeltaMeasurement {
            channel_row: 0,
            start_us: 10.0,
            end_us: 10.0,
            end_y: 35.0,
        };

        // Snapping off: the endpoint stays exactly under the pointer even
        // though an edge at 40µs is within the snap distance.
        viewer.edge_delta_measurement = Some(measurement);
        viewer.set_snapping_enabled(false);
        viewer.update_edge_measurement(layout, Some(Pos2::new(39.0, 60.0)));
        let updated = viewer.edge_delta_measurement.expect("measurement runs");
        // Free at the pointer (pixel→time conversion is f32), not pulled to
        // the edge at 40µs that snapping would have selected.
        assert!(
            (updated.end_us - 39.0).abs() < 0.001,
            "endpoint stayed free: {}",
            updated.end_us
        );
        assert_eq!(updated.end_y, 60.0);

        // Measurement off: nothing stays on screen.
        viewer.set_measurements_enabled(false);
        assert!(viewer.edge_delta_measurement.is_none());
    }

    #[test]
    fn endpoint_snaps_on_another_lane_and_is_free_between_edges() {
        let mut viewer = LogicAnalyzerViewer::new();
        viewer.set_channels(vec![
            ChannelSignal {
                index: 0,
                name: "Clock".into(),
                initial: false,
                transitions: vec![(10.0, true)],
            },
            ChannelSignal {
                index: 1,
                name: "Data".into(),
                initial: false,
                transitions: vec![(12.0, true), (40.0, false)],
            },
        ]);
        viewer.visible_start_us = 0.0;
        viewer.visible_span_us = 100.0;
        viewer.edge_delta_measurement = Some(EdgeDeltaMeasurement {
            channel_row: 0,
            start_us: 10.0,
            end_us: 10.0,
            end_y: 15.0,
        });
        let layout = AnalyzerLayout {
            ruler_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 20.0)),
            labels_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(20.0, 100.0)),
            wave_rect: Rect::from_min_max(Pos2::new(0.0, 20.0), Pos2::new(100.0, 100.0)),
            row_height: 30.0,
            trigger_width: 0.0,
            name_col_width: 0.0,
            badge_width: 0.0,
        };

        viewer.update_edge_measurement(layout, Some(Pos2::new(40.0, 60.0)));
        let measurement = viewer.edge_delta_measurement.unwrap();
        assert_eq!(measurement.end_us, 40.0);
        assert_eq!(measurement.end_y, 65.0);

        viewer.update_edge_measurement(layout, Some(Pos2::new(25.0, 60.0)));
        let measurement = viewer.edge_delta_measurement.unwrap();
        assert_eq!(measurement.end_us, 25.0);
        assert_eq!(measurement.end_y, 60.0);
    }
}
