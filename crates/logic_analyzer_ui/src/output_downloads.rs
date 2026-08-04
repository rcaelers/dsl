use std::collections::BTreeSet;

use crate::host_service::{DownloadableOutput, HostService};

const WINDOW_WIDTH: f32 = 620.0;
const WINDOW_HEIGHT: f32 = 260.0;
const FILE_LIST_MIN_HEIGHT: f32 = 72.0;
const FOOTER_HEIGHT: f32 = 38.0;

/// UI-owned selection and presentation state for host-retained graph outputs.
pub(crate) struct OutputDownloadsWindow {
    open: bool,
    selected: BTreeSet<u64>,
    selection_anchor: Option<(u64, bool)>,
    drag_value: Option<bool>,
}

impl OutputDownloadsWindow {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            selected: BTreeSet::new(),
            selection_anchor: None,
            drag_value: None,
        }
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    /// Shows the output-download chooser and returns any user-presentable failures.
    pub(crate) fn show(&mut self, ctx: &egui::Context, host: &mut dyn HostService) -> Vec<String> {
        if !self.open {
            return Vec::new();
        }
        let outputs = host.pending_output_downloads();
        let available = outputs
            .iter()
            .map(|output| output.id)
            .collect::<BTreeSet<_>>();
        self.selected.retain(|id| available.contains(id));
        let pointer_down = ctx.input(|input| input.pointer.primary_down());
        let pointer_pos = ctx.input(|input| input.pointer.interact_pos());
        let shift = ctx.input(|input| input.modifiers.shift);
        if !pointer_down {
            self.drag_value = None;
        }
        if outputs.is_empty() {
            self.open = false;
            return Vec::new();
        }

        let mut failures = Vec::new();
        let mut open = self.open;
        egui::Window::new("Downloads")
            .open(&mut open)
            .resizable([true, true])
            .default_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .min_size([420.0, 180.0])
            .show(ctx, |ui| {
                ui.label("Select completed graph outputs to download.");
                ui.horizontal(|ui| {
                    if ui.button("Select All").clicked() {
                        self.selected.extend(outputs.iter().map(|output| output.id));
                        self.selection_anchor = None;
                    }
                    if ui.button("Clear").clicked() {
                        self.selected.clear();
                        self.selection_anchor = None;
                    }
                    ui.separator();
                    ui.weak(format!("{} file(s) selected", self.selected.len()));
                });
                ui.separator();
                let list_height = (ui.available_height() - FOOTER_HEIGHT).max(FILE_LIST_MIN_HEIGHT);
                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("output-download-list")
                            .num_columns(4)
                            .striped(true)
                            .min_col_width(78.0)
                            .show(ui, |ui| {
                                ui.strong("Select");
                                ui.strong("File");
                                ui.strong("Type");
                                ui.strong("Size");
                                ui.end_row();
                                for (index, output) in outputs.iter().enumerate() {
                                    let checkbox_size = ui.spacing().interact_size.y;
                                    let (checkbox, response) = ui.allocate_exact_size(
                                        egui::Vec2::splat(checkbox_size),
                                        egui::Sense::click_and_drag(),
                                    );
                                    if response.drag_started() {
                                        self.begin_drag(output.id);
                                    } else if response.clicked() {
                                        self.apply_click(&outputs, index, shift);
                                    }
                                    if pointer_down
                                        && self.drag_value.is_some()
                                        && pointer_pos
                                            .is_some_and(|pointer| response.rect.contains(pointer))
                                    {
                                        self.continue_drag(output.id);
                                    }
                                    let selected = self.selected.contains(&output.id);
                                    let visuals = ui.style().interact(&response);
                                    let painter = ui.painter();
                                    painter.rect_filled(
                                        checkbox,
                                        2.0,
                                        ui.visuals().extreme_bg_color,
                                    );
                                    painter.rect_stroke(
                                        checkbox,
                                        2.0,
                                        egui::Stroke::new(1.0, visuals.fg_stroke.color),
                                        egui::StrokeKind::Inside,
                                    );
                                    if selected {
                                        painter.rect_filled(
                                            checkbox.shrink(4.0),
                                            1.0,
                                            ui.visuals().selection.bg_fill,
                                        );
                                    }
                                    response.on_hover_text(
                                        "Click to toggle · Shift-click for range · Drag to paint",
                                    );
                                    ui.label(&output.name);
                                    ui.weak(&output.content_type);
                                    ui.weak(format_byte_count(output.byte_len));
                                    ui.end_row();
                                }
                            });
                    });
                ui.separator();
                let selected = self.selected.iter().copied().collect::<Vec<_>>();
                if ui
                    .add_enabled(
                        !selected.is_empty(),
                        egui::Button::new(format!("Download Selected ({})", selected.len())),
                    )
                    .clicked()
                {
                    for id in selected {
                        if let Err(error) = host.download_output(id) {
                            failures.push(error);
                        } else {
                            self.selected.remove(&id);
                        }
                    }
                }
            });
        self.open = open;
        failures
    }

    fn apply_click(&mut self, outputs: &[DownloadableOutput], index: usize, shift: bool) {
        let Some(output) = outputs.get(index) else {
            return;
        };
        if shift
            && let Some((anchor, value)) = self.selection_anchor
            && let Some(anchor_index) = outputs.iter().position(|item| item.id == anchor)
        {
            let range = anchor_index.min(index)..=anchor_index.max(index);
            for output in &outputs[range] {
                self.set_selected(output.id, value);
            }
            return;
        }
        let value = !self.selected.contains(&output.id);
        self.set_selected(output.id, value);
        self.selection_anchor = Some((output.id, value));
    }

    fn begin_drag(&mut self, id: u64) {
        let value = !self.selected.contains(&id);
        self.set_selected(id, value);
        self.selection_anchor = Some((id, value));
        self.drag_value = Some(value);
    }

    fn continue_drag(&mut self, id: u64) {
        if let Some(value) = self.drag_value {
            self.set_selected(id, value);
        }
    }

    fn set_selected(&mut self, id: u64, selected: bool) {
        if selected {
            self.selected.insert(id);
        } else {
            self.selected.remove(&id);
        }
    }
}

impl Default for OutputDownloadsWindow {
    fn default() -> Self {
        Self::new()
    }
}

fn format_byte_count(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    match bytes {
        bytes if bytes >= GIB => format!("{:.1} GiB", bytes as f64 / GIB as f64),
        bytes if bytes >= MIB => format!("{:.1} MiB", bytes as f64 / MIB as f64),
        bytes if bytes >= KIB => format!("{:.1} KiB", bytes as f64 / KIB as f64),
        bytes => format!("{bytes} B"),
    }
}

#[cfg(test)]
mod output_downloads_tests {
    use std::collections::BTreeSet;

    use super::{OutputDownloadsWindow, format_byte_count};
    use crate::host_service::DownloadableOutput;

    #[test]
    fn byte_counts_use_compact_binary_units() {
        assert_eq!(format_byte_count(24), "24 B");
        assert_eq!(format_byte_count(2_048), "2.0 KiB");
        assert_eq!(format_byte_count(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn shift_click_extends_the_anchor_selection_across_output_rows() {
        let outputs = (1..=4)
            .map(|id| DownloadableOutput {
                id,
                name: format!("file-{id}"),
                content_type: "application/octet-stream".to_owned(),
                byte_len: 0,
            })
            .collect::<Vec<_>>();
        let mut window = OutputDownloadsWindow::new();

        window.apply_click(&outputs, 1, false);
        window.apply_click(&outputs, 3, true);

        assert_eq!(window.selected, BTreeSet::from([2, 3, 4]));
    }
}
