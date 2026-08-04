//! Transient toasts are the single place `App` reports one-off events (file
//! loaded/saved, node(s)
//! copied/pasted, a live edit applied or failed) without them pinning a
//! toolbar label forever. Ongoing *state* (such as a run that needs a restart
//! to pick up an edit) stays in the toolbar next to Run/Stop. A failed compile
//! attempt is also recorded here so its diagnostics remain available in the
//! Log panel after the toolbar summary changes.

use std::cmp::Reverse;

use egui::{Color32, Context, Ui};
use web_time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Severity {
    Info,
    Warning,
    Error,
}

struct Toast {
    text: String,
    severity: Severity,
    /// Stamped lazily on the first `show()` after the toast is pushed, so
    /// callers never need to thread `egui::Context` through just to report
    /// an event — only `show()` (called once per frame) needs it.
    created: Option<f64>,
    dismissed: bool,
}

struct ToastHistoryEntry {
    text: String,
    source: String,
    time: String,
    timestamp_seconds: u64,
    sequence: u64,
    severity: Severity,
}

/// A user-facing origin for a notification in the Log panel.
pub(crate) enum ToastSource {
    Global,
    Panel(String),
    Node(String),
    Socket { node: String, socket: String },
}

impl ToastSource {
    pub(crate) fn panel(name: impl Into<String>) -> Self {
        Self::Panel(name.into())
    }

    pub(crate) fn node(name: impl Into<String>) -> Self {
        Self::Node(name.into())
    }

    pub(crate) fn socket(node: impl Into<String>, socket: impl Into<String>) -> Self {
        Self::Socket {
            node: node.into(),
            socket: socket.into(),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Global => "Global".to_owned(),
            Self::Panel(name) => format!("Panel: {name}"),
            Self::Node(name) => format!("Node: {name}"),
            Self::Socket { node, socket } => format!("Socket: {node} / {socket}"),
        }
    }
}

/// Info toasts fade out this many seconds after appearing.
const FADE_AFTER_S: f64 = 4.0;
/// The fade is a linear alpha ramp over this final stretch.
const FADE_RAMP_S: f64 = 1.0;
#[derive(Default)]
pub(crate) struct Toasts {
    active: Vec<Toast>,
    history: Vec<ToastHistoryEntry>,
    history_sequence: u64,
}

impl Toasts {
    /// Fades out on its own after ~4s.
    pub(crate) fn info(&mut self, text: impl Into<String>) {
        self.info_from(ToastSource::Global, text);
    }

    pub(crate) fn info_from(&mut self, source: ToastSource, text: impl Into<String>) {
        self.push(Severity::Info, text.into(), source);
    }

    /// Persists until dismissed (✕) or the toast stack scrolls it away.
    pub(crate) fn error(&mut self, text: impl Into<String>) {
        self.error_from(ToastSource::Global, text);
    }

    pub(crate) fn error_from(&mut self, source: ToastSource, text: impl Into<String>) {
        self.push(Severity::Error, text.into(), source);
    }

    /// Persists until dismissed, without presenting a successful operation as a failure.
    pub(crate) fn warning(&mut self, text: impl Into<String>) {
        self.warning_from(ToastSource::Global, text);
    }

    pub(crate) fn warning_from(&mut self, source: ToastSource, text: impl Into<String>) {
        self.push(Severity::Warning, text.into(), source);
    }

    fn push(&mut self, severity: Severity, text: String, source: ToastSource) {
        let (timestamp_seconds, time) = current_time();
        self.active.push(Toast {
            text: text.clone(),
            severity,
            created: None,
            dismissed: false,
        });
        self.history.push(ToastHistoryEntry {
            text,
            source: source.label(),
            time,
            timestamp_seconds,
            sequence: self.history_sequence,
            severity,
        });
        self.history_sequence = self.history_sequence.saturating_add(1);
    }

    fn history_display_order(&self) -> Vec<&ToastHistoryEntry> {
        let mut entries = self.history.iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| (Reverse(entry.timestamp_seconds), entry.sequence));
        entries
    }

    /// Draws the durable in-session notification history for the Log panel.
    pub(crate) fn show_history(&self, ui: &mut Ui) {
        if self.history.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("No messages yet").weak());
            });
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for entry in self.history_display_order() {
                    let (label, color) = severity_presentation(entry.severity);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.weak(&entry.time);
                            ui.colored_label(color, label);
                            ui.weak(&entry.source);
                        });
                        ui.label(&entry.text);
                    });
                    ui.add_space(4.0);
                }
            });
    }

    /// Draws the toast stack bottom-right and prunes expired/dismissed
    /// entries. Call once per frame; cheap no-op when nothing's pending.
    pub(crate) fn show(&mut self, ctx: &Context) {
        if self.active.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        for toast in &mut self.active {
            if toast.created.is_none() {
                toast.created = Some(now);
            }
        }
        self.active.retain(|toast| {
            !toast.dismissed
                && (toast.severity != Severity::Info
                    || now - toast.created.unwrap_or(now) < FADE_AFTER_S)
        });
        if self.active.is_empty() {
            return;
        }

        let mut dismiss: Option<usize> = None;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    for (index, toast) in self.active.iter().enumerate().rev() {
                        let elapsed = now - toast.created.unwrap_or(now);
                        let alpha = if toast.severity != Severity::Info {
                            1.0
                        } else {
                            ((FADE_AFTER_S - elapsed) / FADE_RAMP_S).clamp(0.0, 1.0) as f32
                        };
                        let (bg, fg) = match toast.severity {
                            Severity::Info => (
                                Color32::from_rgba_unmultiplied(45, 45, 45, (alpha * 235.0) as u8),
                                Color32::from_rgba_unmultiplied(
                                    220,
                                    220,
                                    220,
                                    (alpha * 255.0) as u8,
                                ),
                            ),
                            Severity::Warning => (
                                Color32::from_rgb(91, 67, 27),
                                Color32::from_rgb(245, 222, 177),
                            ),
                            Severity::Error => (
                                Color32::from_rgb(92, 38, 38),
                                Color32::from_rgb(240, 210, 210),
                            ),
                        };
                        egui::Frame::new()
                            .fill(bg)
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin {
                                left: 10,
                                right: 8,
                                top: 6,
                                bottom: 6,
                            })
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Notifications acknowledge an action; they are not an
                                    // editable or copyable text surface. In particular, keeping
                                    // them non-selectable prevents a visible diagnostic or the
                                    // copy confirmation itself from taking over the browser
                                    // clipboard while a graph copy command is being handled.
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&toast.text).color(fg),
                                        )
                                        .selectable(false),
                                    );
                                    if toast.severity != Severity::Info
                                        && ui
                                            .add(egui::Button::new("✕").small().frame(false))
                                            .clicked()
                                    {
                                        dismiss = Some(index);
                                    }
                                });
                            });
                        ui.add_space(4.0);
                    }
                });
            });
        if let Some(index) = dismiss {
            self.active[index].dismissed = true;
        }

        let any_fading = self.active.iter().any(|t| t.severity == Severity::Info);
        ctx.request_repaint_after(std::time::Duration::from_millis(if any_fading {
            16
        } else {
            250
        }));
    }
}

fn current_time() -> (u64, String) {
    let timestamp_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = timestamp_seconds % 86_400;
    (
        timestamp_seconds,
        format!(
            "{:02}:{:02}:{:02} UTC",
            seconds / 3_600,
            (seconds / 60) % 60,
            seconds % 60,
        ),
    )
}

fn severity_presentation(severity: Severity) -> (&'static str, Color32) {
    match severity {
        Severity::Info => ("Info", Color32::from_rgb(190, 190, 190)),
        Severity::Warning => ("Warning", Color32::from_rgb(245, 222, 177)),
        Severity::Error => ("Error", Color32::from_rgb(240, 160, 160)),
    }
}

#[cfg(test)]
mod toast_tests {
    use super::{Severity, ToastHistoryEntry, ToastSource, Toasts};

    #[test]
    fn history_retains_expiring_and_dismissible_toasts_with_their_source() {
        let mut toasts = Toasts::default();
        toasts.info("capture loaded");
        toasts.warning_from(ToastSource::panel("Triggers"), "capture warning");
        toasts.error_from(ToastSource::socket("SPI decoder", "MOSI"), "capture failed");

        assert_eq!(toasts.history.len(), 3);
        assert_eq!(toasts.history[0].text, "capture loaded");
        assert_eq!(toasts.history[0].source, "Global");
        assert_eq!(toasts.history[1].severity, Severity::Warning);
        assert_eq!(toasts.history[1].source, "Panel: Triggers");
        assert_eq!(toasts.history[2].source, "Socket: SPI decoder / MOSI");
        assert!(toasts.history[2].time.ends_with(" UTC"));
    }

    #[test]
    fn history_keeps_creation_order_for_entries_with_the_same_second() {
        let toasts = Toasts {
            history: vec![
                history_entry("migration one", 100, 0),
                history_entry("migration two", 100, 1),
                history_entry("Loaded graph", 100, 2),
                history_entry("Saved graph", 101, 3),
            ],
            ..Default::default()
        };

        assert_eq!(
            toasts
                .history_display_order()
                .into_iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            [
                "Saved graph",
                "migration one",
                "migration two",
                "Loaded graph"
            ]
        );
    }

    fn history_entry(text: &str, timestamp_seconds: u64, sequence: u64) -> ToastHistoryEntry {
        ToastHistoryEntry {
            text: text.to_owned(),
            source: "Global".to_owned(),
            time: "00:00:00 UTC".to_owned(),
            timestamp_seconds,
            sequence,
            severity: Severity::Info,
        }
    }
}
