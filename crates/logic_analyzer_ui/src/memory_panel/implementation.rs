use std::time::Duration;

use egui::{Color32, RichText};
use web_time::Instant;

use signal_processing::CollectedLaneStorageBacking;

use super::model::{
    CaptureStorageBacking, MemoryPanelSnapshot, MemoryServiceSnapshot, PersistentCacheSnapshot,
    PersistentCacheSnapshotState,
};

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct MemoryPanel {
    snapshot: MemoryPanelSnapshot,
    last_refresh: Option<Instant>,
}

impl MemoryPanel {
    pub(crate) fn refresh_due(&self) -> bool {
        self.last_refresh
            .is_none_or(|last| last.elapsed() >= REFRESH_INTERVAL)
    }

    pub(crate) fn replace_snapshot(&mut self, snapshot: MemoryPanelSnapshot) {
        self.snapshot = snapshot;
        self.last_refresh = Some(Instant::now());
    }

    pub(crate) fn show(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("Services");
                show_service_table(ui, &self.snapshot.services);
                ui.add_space(12.0);
                ui.heading("Signal data");
                show_signal_table(ui, &self.snapshot);
                ui.add_space(12.0);
                ui.heading("Persistent derived cache");
                show_persistent_cache_table(ui, &self.snapshot.persistent_caches);
            });
    }
}

fn show_service_table(ui: &mut egui::Ui, services: &[MemoryServiceSnapshot]) {
    egui::Grid::new("memory-services")
        .striped(true)
        .min_col_width(100.0)
        .show(ui, |ui| {
            table_header(ui, &["Service", "State", "Usage", "Details"]);
            for service in services {
                ui.label(&service.name);
                ui.label(&service.state);
                ui.label(service_usage(service));
                ui.label(RichText::new(&service.detail).weak());
                ui.end_row();
            }
        });
}

fn show_signal_table(ui: &mut egui::Ui, snapshot: &MemoryPanelSnapshot) {
    egui::Grid::new("memory-signals")
        .striped(true)
        .min_col_width(100.0)
        .show(ui, |ui| {
            table_header(
                ui,
                &["Signal", "Payload", "Backing", "Items", "Data", "Index"],
            );
            if let Some(capture) = &snapshot.capture {
                ui.label(&capture.name);
                ui.label(format!("{} channels", capture.channels));
                ui.label(match capture.backing {
                    CaptureStorageBacking::InMemory => "Memory",
                    CaptureStorageBacking::BuildingIndex => "Building index",
                    CaptureStorageBacking::Indexed => "Indexed",
                    CaptureStorageBacking::GrowingIndex => "Growing index",
                    CaptureStorageBacking::MetadataOnly => "Metadata only",
                });
                ui.label(
                    capture
                        .total_samples
                        .map_or_else(|| "—".to_owned(), format_count),
                );
                ui.label(
                    capture
                        .data_bytes
                        .map_or_else(|| "—".to_owned(), format_bytes),
                );
                let index = capture.index_progress.map_or_else(
                    || {
                        capture.index_path.as_ref().map_or_else(
                            || "—".to_owned(),
                            |path| {
                                path.file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy()
                                    .into_owned()
                            },
                        )
                    },
                    |progress| format!("{:.0}%", progress * 100.0),
                );
                let index_response = ui.label(index);
                let index_detail = capture.index_path.as_ref().map_or_else(
                    || capture.status.clone(),
                    |path| format!("{}\n{}", capture.status, path.display()),
                );
                index_response.on_hover_text(index_detail);
                ui.end_row();
            }
            for lane in &snapshot.derived_lanes {
                let storage = lane.storage;
                ui.label(&lane.name);
                ui.label(&lane.payload_id);
                let backing = match storage.backing {
                    CollectedLaneStorageBacking::Memory => "Memory",
                    CollectedLaneStorageBacking::Indexed => "Indexed",
                    CollectedLaneStorageBacking::PersistentCache => "Persistent cache",
                    CollectedLaneStorageBacking::AdapterManaged => "Adapter managed",
                };
                ui.label(if storage.live {
                    format!("{backing} · live")
                } else {
                    backing.to_owned()
                });
                ui.label(
                    storage
                        .retained_items
                        .map_or_else(|| "—".to_owned(), format_count),
                );
                ui.label(format_optional_bytes(
                    storage.resident_bytes,
                    storage.stored_bytes,
                ));
                ui.label(format_index(storage.index_items, storage.index_bytes));
                ui.end_row();
            }
            if snapshot.capture.is_none() && snapshot.derived_lanes.is_empty() {
                ui.label(
                    RichText::new("No signal data is currently retained")
                        .italics()
                        .weak(),
                );
                ui.end_row();
            }
        });
}

fn show_persistent_cache_table(ui: &mut egui::Ui, caches: &[PersistentCacheSnapshot]) {
    if caches.is_empty() {
        ui.label(RichText::new("The current graph has no persistent cache entries").weak());
        return;
    }
    egui::Grid::new("memory-persistent-cache")
        .striped(true)
        .min_col_width(100.0)
        .show(ui, |ui| {
            table_header(
                ui,
                &["Signal owners", "State", "Items", "Data", "Index", "Key"],
            );
            for cache in caches {
                ui.label(cache.owners.join(", "));
                match &cache.state {
                    PersistentCacheSnapshotState::Ready => {
                        ui.label("Ready");
                    }
                    PersistentCacheSnapshotState::Missing => {
                        ui.label(RichText::new("Missing").weak());
                    }
                    PersistentCacheSnapshotState::Unreadable(error) => {
                        ui.label(RichText::new("Unreadable").color(Color32::LIGHT_RED))
                            .on_hover_text(error);
                    }
                }
                ui.label(cache.items.map_or_else(|| "—".to_owned(), format_count));
                ui.label(
                    cache
                        .data_bytes
                        .map_or_else(|| "—".to_owned(), format_bytes),
                );
                ui.label(format_index(cache.index_items, cache.index_bytes));
                ui.label(short_cache_key(&cache.cache_key))
                    .on_hover_text(&cache.repository);
                ui.end_row();
            }
        });
}

fn table_header(ui: &mut egui::Ui, labels: &[&str]) {
    for label in labels {
        ui.strong(*label);
    }
    ui.end_row();
}

fn service_usage(service: &MemoryServiceSnapshot) -> String {
    match (service.used_bytes, service.budget_bytes) {
        (Some(used), Some(budget)) => format!("{} / {}", format_bytes(used), format_bytes(budget)),
        (Some(used), None) => format_bytes(used),
        (None, Some(budget)) => format!("budget {}", format_bytes(budget)),
        (None, None) => "—".to_owned(),
    }
}

fn format_optional_bytes(resident: Option<u64>, stored: Option<u64>) -> String {
    match (resident, stored) {
        (Some(resident), Some(stored)) if stored > 0 => {
            format!(
                "{} RAM · {} stored",
                format_bytes(resident),
                format_bytes(stored)
            )
        }
        (Some(resident), _) => format_bytes(resident),
        (None, Some(stored)) => format_bytes(stored),
        (None, None) => "—".to_owned(),
    }
}

fn format_index(items: Option<u64>, bytes: Option<u64>) -> String {
    match (items, bytes) {
        (Some(items), Some(bytes)) => format!("{} · {}", format_count(items), format_bytes(bytes)),
        (Some(items), None) => format!("{} records", format_count(items)),
        (None, Some(bytes)) => format_bytes(bytes),
        (None, None) => "—".to_owned(),
    }
}

fn format_count(items: u64) -> String {
    match items {
        0..=999 => items.to_string(),
        1_000..=999_999 => format!("{:.1}k", items as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", items as f64 / 1_000_000.0),
        _ => format!("{:.1}G", items as f64 / 1_000_000_000.0),
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        KIB..=1_048_575 => format!("{:.1} KiB", bytes as f64 / KIB as f64),
        MIB..=1_073_741_823 => format!("{:.1} MiB", bytes as f64 / MIB as f64),
        _ => format!("{:.1} GiB", bytes as f64 / GIB as f64),
    }
}

fn short_cache_key(key: &[u8; 32]) -> String {
    key[..6].iter().map(|byte| format!("{byte:02x}")).collect()
}
