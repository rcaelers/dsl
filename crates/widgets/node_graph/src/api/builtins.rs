use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, Sense, Ui, Vec2};
use serde::{Deserialize, Serialize};

use super::control::{FileDialogFilter, FileDialogRequest, InlineControl, InlineControlContext};
use super::socket::{SocketDef, SocketWithControlDef};
use crate::model::SocketShape;

// ── Built-in socket types ─────────────────────────────────────────────────────

/// Boolean configuration socket with a checkbox inline control.
pub struct BoolSocket;
/// Signed integer configuration socket with a bounded numeric control.
pub struct IntSocket;
/// Floating-point configuration socket with a bounded numeric control.
pub struct FloatSocket;
/// String configuration socket with a text inline control.
pub struct StrSocket;
/// File-path configuration socket with host file-dialog support.
pub struct FileSocket;
/// Wildcard type: accepts (and is accepted by) every other type. Useful as
/// the native type of variadic placeholder inputs and reroute nodes.
pub struct AnySocket;

impl SocketDef for AnySocket {
    type Value = ();

    fn type_name() -> &'static str {
        "Any"
    }
    fn color() -> Color32 {
        Color32::from_rgb(150, 150, 150)
    }
}

// Builtin config sockets follow the graph-wide styling axes:
// square = static config, and the hue is the payload family shared with the
// stream types (green logic, blue integer, violet float, rose text, tan file).
// Red is reserved for error feedback, grey for the wildcard.

impl SocketDef for BoolSocket {
    type Value = bool;

    fn type_name() -> &'static str {
        "Bool"
    }
    fn color() -> Color32 {
        Color32::from_rgb(95, 175, 95)
    }
    fn shape() -> SocketShape {
        SocketShape::Square
    }
}

impl SocketDef for IntSocket {
    type Value = i32;

    fn type_name() -> &'static str {
        "Int"
    }
    fn color() -> Color32 {
        Color32::from_rgb(95, 145, 210)
    }
    fn shape() -> SocketShape {
        SocketShape::Square
    }
}

impl SocketDef for FloatSocket {
    type Value = f32;

    fn type_name() -> &'static str {
        "Float"
    }
    fn color() -> Color32 {
        Color32::from_rgb(165, 130, 215)
    }
    fn shape() -> SocketShape {
        SocketShape::Square
    }
}

impl SocketDef for StrSocket {
    type Value = String;

    fn type_name() -> &'static str {
        "String"
    }
    fn color() -> Color32 {
        Color32::from_rgb(215, 150, 170)
    }
    fn shape() -> SocketShape {
        SocketShape::Square
    }
}

impl SocketDef for FileSocket {
    type Value = String;

    fn type_name() -> &'static str {
        "File"
    }
    fn color() -> Color32 {
        Color32::from_rgb(170, 145, 95)
    }
    fn shape() -> SocketShape {
        SocketShape::Square
    }
}

impl SocketWithControlDef for BoolSocket {
    type Control = BoolValue;
}

impl SocketWithControlDef for IntSocket {
    type Control = IntValue;
}

impl SocketWithControlDef for FloatSocket {
    type Control = FloatValue;
}

impl SocketWithControlDef for StrSocket {
    type Control = StringValue;
}

impl SocketWithControlDef for FileSocket {
    type Control = FileValue;
}

// ── Built-in value types ──────────────────────────────────────────────────────

/// Integer value and optional bounds edited by an inline numeric control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntValue {
    /// Current value.
    pub value: i32,
    /// Inclusive lower bound.
    pub min: i32,
    /// Inclusive upper bound.
    pub max: i32,
}

impl IntValue {
    /// Creates an integer control value with inclusive bounds.
    ///
    /// # Parameters
    /// - `value`: Initial value.
    /// - `min`: Inclusive lower bound.
    /// - `max`: Inclusive upper bound.
    pub fn new(value: i32, min: i32, max: i32) -> Self {
        Self { value, min, max }
    }
    /// Creates an integer control value without practical bounds.
    ///
    /// # Parameters
    /// - `value`: Initial integer value.
    pub fn plain(value: i32) -> Self {
        Self {
            value,
            min: i32::MIN,
            max: i32::MAX,
        }
    }
}

impl InlineControl for IntValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        _context: &mut InlineControlContext<'_>,
    ) -> bool {
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let drag = if resp.dragged() {
            resp.drag_delta().x
        } else {
            0.0
        };
        let old = self.value;
        if drag.abs() > 0.01 {
            self.value = (self.value as f32 + drag * 0.1).round() as i32;
            if self.min != i32::MIN || self.max != i32::MAX {
                self.value = self.value.clamp(self.min, self.max);
            }
        }
        let fill = if self.max > self.min && self.max != i32::MAX {
            Some((self.value - self.min) as f32 / (self.max - self.min) as f32)
        } else {
            None
        };
        paint_number_btn(
            &ui.painter().with_clip_rect(clip_rect),
            rect,
            label,
            &self.value.to_string(),
            fill,
            zoom,
        );
        self.value != old
    }
}

/// Floating-point value, bounds, and drag speed for an inline numeric control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatValue {
    /// Current value.
    pub value: f32,
    /// Inclusive lower bound.
    pub min: f32,
    /// Inclusive upper bound.
    pub max: f32,
    /// Value delta per unit of drag.
    pub speed: f32,
}

impl FloatValue {
    /// Creates a floating-point control value with explicit bounds and drag speed.
    pub fn new(value: f32, min: f32, max: f32, speed: f32) -> Self {
        Self {
            value,
            min,
            max,
            speed,
        }
    }
    /// Returns this value configured with range.
    pub fn with_range(value: f32, min: f32, max: f32) -> Self {
        let speed = if max > min { (max - min) / 100.0 } else { 0.01 };
        Self {
            value,
            min,
            max,
            speed,
        }
    }
    /// Creates a floating-point control value without practical bounds.
    ///
    /// # Parameters
    /// - `value`: Initial floating-point value.
    pub fn plain(value: f32) -> Self {
        Self {
            value,
            min: f32::NEG_INFINITY,
            max: f32::INFINITY,
            speed: 0.01,
        }
    }
}

impl InlineControl for FloatValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        _context: &mut InlineControlContext<'_>,
    ) -> bool {
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let drag = if resp.dragged() {
            resp.drag_delta().x
        } else {
            0.0
        };
        let old = self.value.to_bits();
        if drag.abs() > 0.01 {
            self.value += drag * self.speed;
            if self.min.is_finite() && self.max.is_finite() {
                self.value = self.value.clamp(self.min, self.max);
            }
        }
        let fill = if self.min.is_finite() && self.max.is_finite() && self.max > self.min {
            Some((self.value - self.min) / (self.max - self.min))
        } else {
            None
        };
        paint_number_btn(
            &ui.painter().with_clip_rect(clip_rect),
            rect,
            label,
            &fmt_float(self.value),
            fill,
            zoom,
        );
        self.value.to_bits() != old
    }
}

/// Boolean value edited by an inline checkbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoolValue {
    /// Current checkbox state.
    pub value: bool,
}

impl BoolValue {
    /// Creates a boolean control value.
    pub fn new(value: bool) -> Self {
        Self { value }
    }
}

impl InlineControl for BoolValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        _context: &mut InlineControlContext<'_>,
    ) -> bool {
        let old = self.value;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::LEFT)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.style_mut().spacing.item_spacing = Vec2::splat(2.0 * zoom);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.value, label);
                });
            },
        );
        self.value != old
    }
}

/// String value edited by an inline text field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringValue {
    /// Current text value.
    pub value: String,
}

impl StringValue {
    /// Creates a string control value.
    ///
    /// # Parameters
    /// - `value`: Initial text value.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl InlineControl for StringValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        _context: &mut InlineControlContext<'_>,
    ) -> bool {
        let old = self.value.clone();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::LEFT)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.style_mut().spacing.item_spacing = Vec2::splat(2.0 * zoom);
                ui.add(
                    egui::TextEdit::singleline(&mut self.value)
                        .hint_text(label)
                        .desired_width(rect.width() - 4.0 * zoom),
                );
            },
        );
        self.value != old
    }
}

/// File selection value and host-picker configuration for an inline control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileValue {
    /// Selected path or host-owned file identifier.
    pub value: String,
    #[serde(default)]
    /// Title shown by the host file picker.
    pub dialog_title: String,
    #[serde(default)]
    /// File-type filters offered by the host picker.
    pub filters: Vec<FileDialogFilter>,
    /// Browse with a *save* dialog (pick a new/overwrite target) instead of
    /// an *open* dialog (pick an existing file).
    #[serde(default)]
    pub save: bool,
    #[serde(skip)]
    /// Last user-presentable picker or import error.
    pub dialog_error: Option<String>,
}

impl FileValue {
    /// Creates an open-file control with the default picker title and no filters.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            dialog_title: "Select file".to_string(),
            filters: Vec::new(),
            save: false,
            dialog_error: None,
        }
    }

    /// A picker whose browse button opens a save dialog.
    pub fn new_save(value: impl Into<String>, dialog_title: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            dialog_title: dialog_title.into(),
            filters: Vec::new(),
            save: true,
            dialog_error: None,
        }
    }

    /// Returns this value configured with filter.
    pub fn with_filter(
        value: impl Into<String>,
        dialog_title: impl Into<String>,
        filter_name: impl Into<String>,
        extensions: &[&str],
    ) -> Self {
        Self {
            value: value.into(),
            dialog_title: dialog_title.into(),
            filters: vec![FileDialogFilter {
                name: filter_name.into(),
                extensions: extensions
                    .iter()
                    .map(|extension| extension.to_string())
                    .collect(),
            }],
            save: false,
            dialog_error: None,
        }
    }
}

impl InlineControl for FileValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        context: &mut InlineControlContext<'_>,
    ) -> bool {
        let old = self.value.clone();
        let request_id = ui.id().with(("file-dialog", label)).value();
        if let Some(result) = context.take_picked_file(request_id) {
            match result {
                Ok(path) => {
                    self.value = path;
                    self.dialog_error = None;
                }
                Err(error) => self.dialog_error = Some(error.to_string()),
            }
        }
        let progress = context.picked_file_progress(request_id);

        let pointer_position = ui.input(|input| input.pointer.hover_pos());
        let accepts_drop = pointer_position.is_some_and(|position| rect.contains(position));
        let dropped = accepts_drop
            .then(|| ui.input(|input| input.raw.dropped_files.first().cloned()))
            .flatten();
        if let Some(file) = dropped {
            let accepted = self.filters.is_empty()
                || self.filters.iter().any(|filter| {
                    filter.extensions.iter().any(|extension| {
                        file.name
                            .rsplit_once('.')
                            .is_some_and(|(_, actual)| actual.eq_ignore_ascii_case(extension))
                    })
                });
            if accepted {
                match context.import_dropped_file(super::control::DroppedFile {
                    name: file.name,
                    path: file.path,
                    bytes: file.bytes,
                }) {
                    Ok(path) => {
                        self.value = path;
                        self.dialog_error = None;
                    }
                    Err(error) => self.dialog_error = Some(error.to_string()),
                }
            } else {
                self.dialog_error = Some("the dropped file does not match this input".to_owned());
            }
        }

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.style_mut().spacing.item_spacing = Vec2::splat(2.0 * zoom);
                let button_width = 28.0 * zoom;
                let content_width = (rect.width() - button_width - 6.0 * zoom).max(24.0 * zoom);
                if let Some(progress) = progress {
                    let fraction = progress
                        .total_bytes
                        .filter(|total| *total > 0)
                        .map_or(0.0, |total| progress.completed_bytes as f32 / total as f32);
                    let text = progress.total_bytes.map_or_else(
                        || "Selecting…".to_owned(),
                        |total| {
                            format!(
                                "Importing {:.0}% · {} / {} MiB",
                                fraction * 100.0,
                                progress.completed_bytes / (1024 * 1024),
                                total.div_ceil(1024 * 1024)
                            )
                        },
                    );
                    ui.add_sized(
                        [content_width, rect.height()],
                        egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).text(text),
                    );
                    if ui.button("×").on_hover_text("Cancel import").clicked() {
                        context.cancel_picked_file(request_id);
                    }
                } else {
                    let text = ui.add(
                        egui::TextEdit::singleline(&mut self.value)
                            .hint_text(label)
                            .desired_width(content_width),
                    );
                    if text.changed() {
                        self.dialog_error = None;
                    }
                    if let Some(error) = &self.dialog_error {
                        text.on_hover_text(error);
                    }
                    if ui
                        .add_enabled(
                            context.file_dialog_available(self.save),
                            egui::Button::new("…"),
                        )
                        .clicked()
                        && let Some(path) = context.pick_file(FileDialogRequest {
                            request_id,
                            title: &self.dialog_title,
                            filters: &self.filters,
                            save: self.save,
                        })
                    {
                        self.value = path;
                        self.dialog_error = None;
                    }
                }
            },
        );
        self.value != old
    }
}

/// Selected index and choices for an inline enumeration control.
#[derive(Debug, Clone)]
pub struct EnumValue {
    /// Index of the selected variant.
    pub index: usize,
    /// User-facing variant names in selection order.
    pub variants: Vec<String>,
}

impl EnumValue {
    /// Creates an enumeration control value from borrowed variant names.
    ///
    /// # Parameters
    /// - `index`: Initially selected variant index.
    /// - `variants`: User-facing choices in selection order.
    pub fn new(index: usize, variants: &[&str]) -> Self {
        Self {
            index,
            variants: variants.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The currently selected variant name ("" when out of range).
    pub fn selected(&self) -> &str {
        self.variants.get(self.index).map_or("", String::as_str)
    }

    /// Selects `name` if it is a known variant; ignores unknown names.
    pub fn select(&mut self, name: &str) {
        if let Some(index) = self.variants.iter().position(|variant| variant == name) {
            self.index = index;
        }
    }
}

/// Persisted by variant *name*, not index, so saved graphs survive variant
/// reorders in node defs. Legacy files that stored only an index still load.
impl Serialize for EnumValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("EnumValue", 2)?;
        s.serialize_field("value", self.selected())?;
        s.serialize_field("variants", &self.variants)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for EnumValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Repr {
            #[serde(default)]
            value: Option<String>,
            #[serde(default)]
            index: Option<usize>,
            #[serde(default)]
            variants: Vec<String>,
        }
        let repr = Repr::deserialize(deserializer)?;
        let index = repr
            .value
            .and_then(|name| repr.variants.iter().position(|variant| *variant == name))
            .or(repr.index)
            .unwrap_or(0)
            .min(repr.variants.len().saturating_sub(1));
        Ok(Self {
            index,
            variants: repr.variants,
        })
    }
}

impl InlineControl for EnumValue {
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        _context: &mut InlineControlContext<'_>,
    ) -> bool {
        let old = self.index;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::LEFT)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip_rect));
                ui.style_mut().spacing.item_spacing = Vec2::splat(2.0 * zoom);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).size(10.0 * zoom));
                    let selected = self.variants.get(self.index).cloned().unwrap_or_default();
                    let vars = self.variants.clone();
                    let mut new_idx = self.index;
                    egui::ComboBox::from_id_salt(egui::Id::new(("enum_val", label)))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (vi, variant) in vars.iter().enumerate() {
                                if ui.selectable_label(new_idx == vi, variant).clicked() {
                                    new_idx = vi;
                                }
                            }
                        });
                    self.index = new_idx;
                });
            },
        );
        self.index != old
    }
}

// ── Shared rendering helpers ──────────────────────────────────────────────────

fn fmt_float(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e6 {
        format!("{:.0}", v)
    } else {
        format!("{:.3}", v)
    }
}

fn paint_number_btn(
    painter: &egui::Painter,
    rect: Rect,
    label: &str,
    value: &str,
    fill_ratio: Option<f32>,
    zoom: f32,
) {
    let rounding = CornerRadius::same(3);
    painter.rect_filled(rect, rounding, Color32::from_rgb(56, 56, 56));
    if let Some(ratio) = fill_ratio {
        let ratio = ratio.clamp(0.0, 1.0);
        if ratio > 0.001 {
            let fill_rect =
                Rect::from_min_size(rect.min, Vec2::new(rect.width() * ratio, rect.height()));
            painter.rect_filled(
                fill_rect,
                rounding,
                Color32::from_rgba_unmultiplied(61, 133, 224, 120),
            );
        }
    }
    let text_color = Color32::from_rgb(210, 210, 210);
    let font = FontId::proportional((11.0 * zoom).clamp(7.0, 14.0));
    painter.text(
        Pos2::new(rect.left() + 5.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font.clone(),
        text_color,
    );
    painter.text(
        Pos2::new(rect.right() - 5.0, rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        font,
        text_color,
    );
}

#[cfg(test)]
mod builtins_tests {
    use egui::Context;

    use super::super::control::{
        FileDialogError, FileDialogRequest, FileDialogService, InlineControl, InlineControlContext,
    };
    use super::FileValue;

    struct CompletingFileDialog {
        completion: Option<Result<String, FileDialogError>>,
    }

    impl FileDialogService for CompletingFileDialog {
        fn available(&self, _save: bool) -> bool {
            true
        }

        fn pick(&mut self, _request: FileDialogRequest<'_>) -> Option<String> {
            None
        }

        fn take_picked(&mut self, _request_id: u64) -> Option<Result<String, FileDialogError>> {
            self.completion.take()
        }
    }

    #[test]
    fn file_control_consumes_a_selection_from_the_injected_dialog() {
        let context = Context::default();
        let mut dialog = CompletingFileDialog {
            completion: Some(Ok("capture.dsl".to_owned())),
        };
        let mut value = FileValue::new("");
        let mut changed = false;

        let _ = context.run_ui(Default::default(), |ui| {
            let rect = ui.available_rect_before_wrap();
            let mut control_context = InlineControlContext::new(&mut dialog);
            changed = value.draw_widget(ui, "Capture", rect, 1.0, rect, &mut control_context);
        });

        assert!(changed);
        assert_eq!(value.value, "capture.dsl");
    }
}
