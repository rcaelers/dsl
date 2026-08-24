use egui::{Color32, CursorIcon, FontId, PointerButton, Pos2, Rect, Response, Ui};

use crate::types::AnalyzerLayout;
use crate::viewer::LogicAnalyzerViewer;

/// A persisted, host-owned point displayed on the shared timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineMarker {
    /// Host-owned stable marker identifier.
    pub id: String,
    /// User-facing marker label.
    pub label: String,
    /// Position in the viewer's microsecond time domain.
    pub time_us: f64,
}

/// A completed marker gesture for the host to apply to its document.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineMarkerEdit {
    /// Host-owned stable marker identifier.
    pub id: String,
    /// New marker position in the viewer's microsecond time domain.
    pub time_us: f64,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct TimelineMarkerInput {
    pub(crate) active: Option<usize>,
    pub(crate) blocks_pan: bool,
}

impl LogicAnalyzerViewer {
    /// Replaces host-owned markers while preserving the position of a marker
    /// currently being dragged until the host receives the completed edit.
    ///
    /// # Parameters
    /// - `markers`: Input consumed by this operation.
    pub fn set_timeline_markers(&mut self, mut markers: Vec<TimelineMarker>) {
        markers.retain(|marker| marker.time_us.is_finite() && marker.time_us >= 0.0);
        let active_id = self
            .drag_timeline_marker
            .and_then(|index| self.timeline_markers.get(index))
            .map(|marker| (marker.id.clone(), marker.time_us));
        if let Some((active_id, active_time_us)) = active_id {
            if let Some(marker) = markers.iter_mut().find(|marker| marker.id == active_id) {
                marker.time_us = active_time_us;
                self.drag_timeline_marker =
                    markers.iter().position(|marker| marker.id == active_id);
            } else {
                self.drag_timeline_marker = None;
                self.timeline_marker_drag_changed = false;
            }
        }
        self.timeline_markers = markers;
    }

    /// Sets timeline marker editing enabled.
    pub fn set_timeline_marker_editing_enabled(&mut self, enabled: bool) {
        self.timeline_marker_editing_enabled = enabled;
        if !enabled {
            self.drag_timeline_marker = None;
            self.timeline_marker_drag_changed = false;
        }
    }

    /// Takes timeline marker edit, leaving its default state.
    pub fn take_timeline_marker_edit(&mut self) -> Option<TimelineMarkerEdit> {
        self.pending_timeline_marker_edit.take()
    }

    pub(crate) fn handle_timeline_marker_input(
        &mut self,
        ui: &Ui,
        response: &Response,
        layout: AnalyzerLayout,
    ) -> TimelineMarkerInput {
        let mut state = TimelineMarkerInput::default();
        if layout.wave_rect.width() <= 1.0 || self.timeline_markers.is_empty() {
            self.drag_timeline_marker = None;
            return state;
        }

        let pointer = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|input| input.pointer.hover_pos()))
            // Marker lines stay grabbable a few points past the wave area,
            // which is where the scrollbar column sits; the scrollbar owns it.
            .filter(|pointer| !self.pointer_over_scrollbar(layout, *pointer));
        let flags = self.timeline_marker_flag_layout(ui, layout.wave_rect, layout.ruler_rect);
        let hovered = pointer.and_then(|pointer| {
            self.timeline_marker_at_pointer(layout.wave_rect, layout.ruler_rect, &flags, pointer)
        });
        let drag_button = self
            .input_bindings
            .pointer_button(
                &[
                    "logic_analyzer.timeline_marker",
                    "logic_analyzer.timeline",
                    "logic_analyzer",
                ],
                "drag_cursor",
            )
            .unwrap_or(PointerButton::Primary);
        if self.timeline_marker_editing_enabled && response.drag_started_by(drag_button) {
            let grab_pos = ui.input(|input| input.pointer.press_origin()).or(pointer);
            self.drag_timeline_marker = grab_pos.and_then(|position| {
                self.timeline_marker_at_pointer(
                    layout.wave_rect,
                    layout.ruler_rect,
                    &flags,
                    position,
                )
            });
            self.timeline_marker_drag_changed = false;
        }

        if let Some(index) = self.drag_timeline_marker {
            if response.dragged_by(drag_button) {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let raw_time_us = self.x_to_time(layout.wave_rect, pointer.x).max(0.0);
                    let time_us = self.snap_cursor_time(layout.wave_rect, pointer, raw_time_us);
                    if let Some(marker) = self.timeline_markers.get_mut(index)
                        && marker.time_us.to_bits() != time_us.to_bits()
                    {
                        marker.time_us = time_us;
                        self.timeline_marker_drag_changed = true;
                    }
                }
                state.blocks_pan = true;
            } else if ui.input(|input| input.pointer.button_released(drag_button)) {
                if self.timeline_marker_drag_changed
                    && let Some(marker) = self.timeline_markers.get(index)
                {
                    self.pending_timeline_marker_edit = Some(TimelineMarkerEdit {
                        id: marker.id.clone(),
                        time_us: marker.time_us,
                    });
                }
                self.drag_timeline_marker = None;
                self.timeline_marker_drag_changed = false;
            }
        }

        state.active = self.drag_timeline_marker.or(hovered);
        if state.active.is_some() {
            self.hovered_input_context = "logic_analyzer.timeline_marker";
            ui.ctx()
                .set_cursor_icon(if self.timeline_marker_editing_enabled {
                    CursorIcon::ResizeHorizontal
                } else {
                    CursorIcon::NotAllowed
                });
        }
        state
    }

    fn timeline_marker_flag_layout(&self, ui: &Ui, wave_rect: Rect, ruler_rect: Rect) -> Vec<Rect> {
        self.timeline_markers
            .iter()
            .map(|marker| {
                let x = self.time_to_x_unclamped(wave_rect, marker.time_us);
                let label = timeline_marker_label(marker);
                let width = ui.ctx().fonts_mut(|fonts| {
                    fonts
                        .layout_no_wrap(label, FontId::proportional(10.0), Color32::BLACK)
                        .size()
                        .x
                });
                timeline_marker_flag_geometry(x, ruler_rect, width)
            })
            .collect()
    }

    fn timeline_marker_at_pointer(
        &self,
        wave_rect: Rect,
        ruler_rect: Rect,
        flags: &[Rect],
        pointer: Pos2,
    ) -> Option<usize> {
        const LINE_HIT_PX: f32 = 6.0;
        if let Some(index) = flags.iter().position(|flag| flag.contains(pointer)) {
            return Some(index);
        }
        if pointer.y < ruler_rect.top()
            || pointer.y > wave_rect.bottom()
            || pointer.x < wave_rect.left() - LINE_HIT_PX
            || pointer.x > wave_rect.right() + LINE_HIT_PX
        {
            return None;
        }
        self.timeline_markers
            .iter()
            .enumerate()
            .map(|(index, marker)| {
                let x = self.time_to_x_unclamped(wave_rect, marker.time_us);
                (index, (pointer.x - x).abs())
            })
            .filter(|(_, distance)| *distance <= LINE_HIT_PX)
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(index, _)| index)
    }
}

pub(crate) fn timeline_marker_color() -> Color32 {
    Color32::from_rgb(245, 150, 55)
}

pub(crate) fn timeline_marker_label(marker: &TimelineMarker) -> String {
    format!("{}  {}", marker.label, format_marker_time(marker.time_us))
}

pub(crate) fn timeline_marker_flag_geometry(x: f32, ruler_rect: Rect, label_width: f32) -> Rect {
    const HEIGHT: f32 = 15.0;
    let width = label_width + 12.0;
    let left = (x - width * 0.5).clamp(
        ruler_rect.left(),
        (ruler_rect.right() - width).max(ruler_rect.left()),
    );
    Rect::from_min_size(
        Pos2::new(left, ruler_rect.bottom() - HEIGHT - 1.0),
        egui::vec2(width, HEIGHT),
    )
}

fn format_marker_time(time_us: f64) -> String {
    if time_us >= 1_000_000.0 {
        format!("{:.6}s", time_us / 1_000_000.0)
    } else if time_us >= 1_000.0 {
        format!("{:.6}ms", time_us / 1_000.0)
    } else {
        format!("{time_us:.3}µs")
    }
}

#[cfg(test)]
mod timeline_marker_tests {
    use super::*;

    #[test]
    fn marker_flag_uses_the_lower_ruler_row() {
        let ruler = Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 34.0));
        let flag = timeline_marker_flag_geometry(200.0, ruler, 80.0);
        assert_eq!(flag.bottom(), 33.0);
        assert!(flag.top() >= 18.0);
    }
}
