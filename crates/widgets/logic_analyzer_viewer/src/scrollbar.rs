//! Vertical row scrolling.
//!
//! The row stack is taller than the viewport whenever lanes are added or made
//! taller, so the viewer reserves a scrollbar column on the right and offsets
//! the row coordinate system (see [`LogicAnalyzerViewer::rows_origin_y`]).
//! The column is reserved only while the rows genuinely overflow: with
//! everything visible there is no scrollbar and the waveform keeps the full
//! width.

use egui::{PointerButton, Pos2, Rect, Response, Ui};

use crate::types::AnalyzerLayout;
use crate::viewer::LogicAnalyzerViewer;

/// Width of the reserved scrollbar column, in points.
pub(crate) const SCROLLBAR_WIDTH: f32 = 10.0;

/// Shortest the thumb may become, so a very long row stack still leaves
/// something grabbable.
const MIN_THUMB_HEIGHT: f32 = 24.0;

/// Where the scrollbar's track and thumb are, and how far the rows can move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollbarGeometry {
    pub(crate) track: Rect,
    pub(crate) thumb: Rect,
    pub(crate) max_offset: f32,
}

impl LogicAnalyzerViewer {
    /// Height the rows overflow the viewport by — the largest useful scroll
    /// offset, and zero when everything fits.
    pub(crate) fn max_scroll_offset(&self, viewport_height: f32, default_height: f32) -> f32 {
        (self.rows_total_height(default_height) - viewport_height).max(0.0)
    }

    /// Keeps the offset inside the scrollable range. Called once per frame:
    /// rows, row heights and the viewport all change independently of any
    /// scroll gesture, so what was a valid offset last frame may not be one
    /// now — shrinking the row stack scrolls back up rather than leaving the
    /// view parked past the last row.
    pub(crate) fn clamp_scroll_offset(&mut self, layout: AnalyzerLayout) {
        let max = self.max_scroll_offset(layout.wave_rect.height(), layout.row_height);
        self.scroll_offset_y = self.scroll_offset_y.clamp(0.0, max);
        if max <= 0.0 {
            self.scrollbar_drag_grab = None;
        }
    }

    /// Whether a pointer is inside the scrollbar column. Line hit zones reach
    /// a few points past the wave area, which is exactly where the scrollbar
    /// now is, so cursor and marker hit-testing exclude it.
    pub(crate) fn pointer_over_scrollbar(&self, layout: AnalyzerLayout, pointer: Pos2) -> bool {
        self.scrollbar_geometry(layout)
            .is_some_and(|geometry| geometry.track.contains(pointer))
    }

    /// Track and thumb for the current row stack, or `None` while every row
    /// fits and no scrollbar is shown.
    pub(crate) fn scrollbar_geometry(&self, layout: AnalyzerLayout) -> Option<ScrollbarGeometry> {
        let wave_rect = layout.wave_rect;
        let viewport_height = wave_rect.height();
        let max_offset = self.max_scroll_offset(viewport_height, layout.row_height);
        if max_offset <= 0.0 || viewport_height <= 1.0 {
            return None;
        }
        // The column `layout` reserved sits immediately right of the waves.
        let track = Rect::from_min_max(
            egui::Pos2::new(wave_rect.right(), wave_rect.top()),
            egui::Pos2::new(wave_rect.right() + SCROLLBAR_WIDTH, wave_rect.bottom()),
        );
        let content_height = self.rows_total_height(layout.row_height);
        let thumb_height = (viewport_height * viewport_height / content_height)
            .clamp(MIN_THUMB_HEIGHT.min(viewport_height), viewport_height);
        // The thumb travels the track's leftover room, so its bottom lands
        // exactly at the track's bottom at maximum scroll however short the
        // minimum-height clamp made it.
        let travel = (viewport_height - thumb_height).max(0.0);
        let progress = if max_offset > 0.0 {
            (self.scroll_offset_y / max_offset).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let thumb_top = track.top() + travel * progress;
        let thumb = Rect::from_min_max(
            egui::Pos2::new(track.left() + 1.0, thumb_top),
            egui::Pos2::new(track.right() - 1.0, thumb_top + thumb_height),
        );
        Some(ScrollbarGeometry {
            track,
            thumb,
            max_offset,
        })
    }

    /// Drives scrollbar dragging for one frame, returning whether the
    /// scrollbar owns the pointer — the caller suppresses panning and the
    /// other pointer gestures while it does.
    ///
    /// Pressing the track above or below the thumb jumps a page in that
    /// direction; pressing the thumb starts a drag that tracks the pointer.
    pub(crate) fn handle_scrollbar_input(
        &mut self,
        ui: &Ui,
        response: &Response,
        layout: AnalyzerLayout,
    ) -> bool {
        let Some(geometry) = self.scrollbar_geometry(layout) else {
            self.scrollbar_drag_grab = None;
            return false;
        };
        let button = PointerButton::Primary;

        if response.drag_started_by(button) || response.clicked_by(button) {
            // Hit-test the press origin: egui reports a drag only once the
            // pointer has moved past the click threshold, by which time it
            // may already have left the narrow thumb.
            let press = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos());
            if let Some(press) = press.filter(|press| geometry.track.contains(*press)) {
                if geometry.thumb.contains(press) {
                    self.scrollbar_drag_grab = Some(press.y - geometry.thumb.top());
                } else {
                    let page = layout.wave_rect.height().max(1.0);
                    let delta = if press.y < geometry.thumb.top() {
                        -page
                    } else {
                        page
                    };
                    self.scroll_offset_y =
                        (self.scroll_offset_y + delta).clamp(0.0, geometry.max_offset);
                    // Grab the thumb where it now is, so continuing into a
                    // drag from the same press keeps following the pointer.
                    self.scrollbar_drag_grab = self
                        .scrollbar_geometry(layout)
                        .map(|updated| (press.y - updated.thumb.top()).max(0.0));
                }
                return true;
            }
        }

        let Some(grab) = self.scrollbar_drag_grab else {
            return false;
        };
        if !response.dragged_by(button) {
            self.scrollbar_drag_grab = None;
            // Still owns this frame's release so the click that ends a drag
            // is not also read as a viewer gesture.
            return response.clicked_by(button);
        }
        if let Some(pointer) = response.interact_pointer_pos() {
            let travel = (geometry.track.height() - geometry.thumb.height()).max(0.0);
            self.scroll_offset_y = if travel > 0.0 {
                let thumb_top = pointer.y - grab;
                let progress = ((thumb_top - geometry.track.top()) / travel).clamp(0.0, 1.0);
                progress * geometry.max_offset
            } else {
                0.0
            };
        }
        true
    }
}

#[cfg(test)]
mod scrollbar_tests {
    use egui::{Pos2, Rect};

    use super::*;
    use crate::{ChannelSignal, LogicAnalyzerViewer};

    /// 80pt of viewport with `count` rows of the default 30pt each.
    fn viewer_with_rows(count: usize) -> (LogicAnalyzerViewer, AnalyzerLayout) {
        let mut viewer = LogicAnalyzerViewer::new();
        viewer.set_channels(
            (0..count)
                .map(|index| ChannelSignal {
                    index,
                    name: format!("Ch {index}"),
                    initial: false,
                    transitions: vec![(10.0, true)],
                })
                .collect(),
        );
        viewer.ensure_row_order();
        let layout = AnalyzerLayout {
            ruler_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(200.0, 20.0)),
            labels_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(20.0, 100.0)),
            wave_rect: Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(200.0, 100.0)),
            row_height: 30.0,
            trigger_width: 0.0,
            name_col_width: 0.0,
            badge_width: 0.0,
        };
        (viewer, layout)
    }

    #[test]
    fn no_scrollbar_while_every_row_fits() {
        let (viewer, layout) = viewer_with_rows(2);

        assert_eq!(viewer.rows_total_height(layout.row_height), 60.0);
        assert!(viewer.scrollbar_geometry(layout).is_none());
        assert_eq!(
            viewer.max_scroll_offset(layout.wave_rect.height(), 30.0),
            0.0
        );
    }

    #[test]
    fn overflowing_rows_get_a_thumb_that_spans_the_track_at_the_extremes() {
        let (mut viewer, layout) = viewer_with_rows(8);
        // 240pt of rows in an 80pt viewport: 160pt of overflow.
        let geometry = viewer.scrollbar_geometry(layout).expect("rows overflow");
        assert_eq!(geometry.max_offset, 160.0);
        assert_eq!(geometry.thumb.top(), geometry.track.top());

        viewer.scroll_offset_y = geometry.max_offset;
        let scrolled = viewer.scrollbar_geometry(layout).expect("rows overflow");
        assert!(
            (scrolled.thumb.bottom() - scrolled.track.bottom()).abs() < 0.001,
            "thumb bottom {} reaches track bottom {}",
            scrolled.thumb.bottom(),
            scrolled.track.bottom()
        );
    }

    #[test]
    fn scrolling_moves_rows_up_and_keeps_hit_testing_aligned() {
        let (mut viewer, layout) = viewer_with_rows(8);
        let top = layout.wave_rect.top();

        assert_eq!(viewer.row_top(top, 0, 30.0), 20.0);
        assert_eq!(viewer.row_at_vertical(top, 25.0, 30.0), Some(0));

        viewer.scroll_offset_y = 30.0;

        // Row 0 is now scrolled out of sight and row 1 sits at the top.
        assert_eq!(viewer.row_top(top, 0, 30.0), -10.0);
        assert_eq!(viewer.row_at_vertical(top, 25.0, 30.0), Some(1));
        // A pointer over the ruler never resolves to a scrolled-up row.
        assert_eq!(viewer.row_at_vertical(top, 10.0, 30.0), None);
    }

    /// The reserved column is what makes the scrollbar "only visible when the
    /// lanes do not fit": `layout` shrinks the wave area for it exactly when
    /// the rows overflow, and gives the width straight back when they don't.
    #[test]
    fn layout_reserves_the_column_only_while_rows_overflow() {
        fn wave_right(viewer: &LogicAnalyzerViewer, height: f32) -> f32 {
            let context = egui::Context::default();
            let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, height));
            context.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            });
            let ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("scrollbar_layout_test"),
                egui::UiBuilder::new().max_rect(rect),
            );
            let right = viewer.layout(&ui, rect).wave_rect.right();
            let mut output = context.end_pass();
            output.textures_delta.clear();
            right
        }

        // Eight 30pt rows need 240pt; a 34pt ruler leaves 266pt of viewport
        // in a 300pt-tall viewer, so everything fits and nothing is reserved.
        let (viewer, _) = viewer_with_rows(8);
        assert_eq!(wave_right(&viewer, 300.0), 400.0);

        // The same rows in a 150pt-tall viewer overflow, and the column is
        // taken out of the wave area rather than drawn over it.
        assert_eq!(wave_right(&viewer, 150.0), 400.0 - SCROLLBAR_WIDTH);
    }

    #[test]
    fn shrinking_the_row_stack_scrolls_back_into_range() {
        let (mut viewer, layout) = viewer_with_rows(8);
        viewer.scroll_offset_y = 160.0;
        viewer.clamp_scroll_offset(layout);
        assert_eq!(viewer.scroll_offset_y, 160.0);

        // Every row removed but two: nothing overflows, so nothing scrolls.
        viewer.set_channels(vec![
            ChannelSignal {
                index: 0,
                name: "Ch 0".into(),
                initial: false,
                transitions: vec![(10.0, true)],
            },
            ChannelSignal {
                index: 1,
                name: "Ch 1".into(),
                initial: false,
                transitions: vec![(10.0, true)],
            },
        ]);
        viewer.ensure_row_order();
        viewer.clamp_scroll_offset(layout);

        assert_eq!(viewer.scroll_offset_y, 0.0);
        assert!(viewer.scrollbar_geometry(layout).is_none());
    }
}
