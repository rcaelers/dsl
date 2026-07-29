//! Viewer presentation for SPI-derived lanes.

use std::fmt::Write;
use std::sync::Arc;

use logic_analyzer_graph_api::node::{
    ProtocolPacketDisplay, ProtocolPacketPresentationRegistration,
};
use logic_analyzer_graph_api::node_support::{
    DecoderTableCellMode, DecoderTableColumnDescriptor, LaneBadgeDescriptor,
    LanePresentationDescriptor,
};
use logic_analyzer_processing::nodes::decoders::spi_decoder::SPI_TRANSACTION_PROTOCOL_ID;
use logic_analyzer_viewer::{
    AnnotationVisual, DefaultViewerLaneRenderer, DerivedLaneId, ViewerLaneGroup,
    ViewerLaneRenderer, ViewerLaneRendererRegistration, ViewerLaneTheme, ViewerLaneTrackId,
};
use signal_processing::{ProtocolPacket, ProtocolValue};

use crate::payloads::WordSnapshotRenderer;

struct SpiLaneRenderer;

impl ViewerLaneRenderer for SpiLaneRenderer {
    fn annotation_visual(
        &self,
        track: &ViewerLaneTrackId,
        _theme: &ViewerLaneTheme,
        value: u64,
        mut default: AnnotationVisual,
    ) -> AnnotationVisual {
        if track.as_str() == "bits" && value <= 1 {
            default.label = value.to_string();
        }
        default
    }

    fn snap_lanes(&self, group: &ViewerLaneGroup, _pointer_fraction: f32) -> Vec<DerivedLaneId> {
        group
            .tracks
            .iter()
            .map(|track| track.lane.clone())
            .collect()
    }
}

pub(crate) fn spi_output_presentation(def_index: usize) -> Option<LanePresentationDescriptor> {
    match def_index {
        2 => Some(LanePresentationDescriptor::new(
            "mosi",
            "bits",
            0,
            1.0,
            LaneBadgeDescriptor::new("O", [215, 140, 60]),
            SPI_WAVEFORM_RENDERER,
        )),
        3 => Some(LanePresentationDescriptor::new(
            "mosi",
            "data",
            1,
            1.0,
            LaneBadgeDescriptor::new("O", [215, 140, 60]),
            SPI_WAVEFORM_RENDERER,
        )),
        4 => Some(LanePresentationDescriptor::new(
            "miso",
            "bits",
            0,
            1.0,
            LaneBadgeDescriptor::new("I", [90, 145, 210]),
            SPI_WAVEFORM_RENDERER,
        )),
        5 => Some(LanePresentationDescriptor::new(
            "miso",
            "data",
            1,
            1.0,
            LaneBadgeDescriptor::new("I", [90, 145, 210]),
            SPI_WAVEFORM_RENDERER,
        )),
        _ => None,
    }
}

pub(crate) fn spi_table_column(def_index: usize) -> Option<DecoderTableColumnDescriptor> {
    if def_index == 6 {
        return Some(DecoderTableColumnDescriptor::new(
            "transactions",
            "transaction",
            "Transaction",
            0,
            true,
            DecoderTableCellMode::Single,
            "transaction",
            SPI_TRANSACTION_TABLE_RENDERER,
        ));
    }
    let (column_key, label, order, row_anchor, mode, track_key) = match def_index {
        2 => (
            "mosi_bits",
            "MOSI Bits",
            0,
            false,
            DecoderTableCellMode::Joined(String::new()),
            "bits",
        ),
        3 => (
            "mosi_data",
            "MOSI Data",
            1,
            true,
            DecoderTableCellMode::Single,
            "data",
        ),
        4 => (
            "miso_bits",
            "MISO Bits",
            2,
            false,
            DecoderTableCellMode::Joined(String::new()),
            "bits",
        ),
        5 => (
            "miso_data",
            "MISO Data",
            3,
            true,
            DecoderTableCellMode::Single,
            "data",
        ),
        _ => return None,
    };
    Some(DecoderTableColumnDescriptor::new(
        "decoder",
        column_key,
        label,
        order,
        row_anchor,
        mode,
        track_key,
        SPI_TABLE_RENDERER,
    ))
}

const SPI_WAVEFORM_RENDERER: &str = "org.logicconduit.renderer.spi-waveform/v1";
const SPI_TABLE_RENDERER: &str = "org.logicconduit.renderer.spi-table/v1";
const SPI_TRANSACTION_TABLE_RENDERER: &str = "org.logicconduit.renderer.spi-transaction-table/v1";

inventory::submit! {
    ProtocolPacketPresentationRegistration::new(
        SPI_TRANSACTION_PROTOCOL_ID,
        spi_transaction_display,
    )
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(SPI_TRANSACTION_TABLE_RENDERER, || {
        Arc::new(DefaultViewerLaneRenderer)
    })
}

fn spi_transaction_display(packet: &ProtocolPacket) -> ProtocolPacketDisplay {
    ProtocolPacketDisplay::new(spi_transaction_label(packet))
}

fn spi_transaction_label(packet: &ProtocolPacket) -> String {
    let ProtocolValue::Mapping(fields) = &packet.value else {
        return packet.display_text();
    };
    let digits = fields
        .get("word_bits")
        .and_then(|value| match value {
            ProtocolValue::Integer(bits) if *bits > 0 => usize::try_from(*bits).ok(),
            _ => None,
        })
        .map_or(2, |bits| bits.div_ceil(4));
    let mut parts = Vec::new();
    for (key, label) in [("mosi", "MOSI"), ("miso", "MISO")] {
        let Some(ProtocolValue::List(words)) = fields.get(key) else {
            continue;
        };
        let mut text = format!("{label} [");
        for (index, word) in words.iter().take(8).enumerate() {
            if index > 0 {
                text.push(' ');
            }
            match word {
                ProtocolValue::Integer(value) if *value >= 0 => {
                    let _ = write!(text, "{value:0digits$X}");
                }
                _ => text.push('?'),
            }
        }
        if words.len() > 8 {
            let _ = write!(text, " … +{}", words.len() - 8);
        }
        text.push(']');
        parts.push(text);
    }
    if fields.get("complete") == Some(&ProtocolValue::Bool(false)) {
        let diagnostics = fields
            .get("diagnostics")
            .and_then(|value| match value {
                ProtocolValue::List(values) => Some(values),
                _ => None,
            })
            .into_iter()
            .flatten()
            .filter_map(|value| match value {
                ProtocolValue::String(diagnostic) => Some(diagnostic.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if diagnostics.is_empty() {
            parts.push("Incomplete".to_owned());
        } else {
            parts.push(format!("Incomplete ({})", diagnostics.join("; ")));
        }
    }
    if parts.is_empty() {
        packet.display_text()
    } else {
        parts.join(" · ")
    }
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(SPI_WAVEFORM_RENDERER, || {
        Arc::new(WordSnapshotRenderer::new(Arc::new(SpiLaneRenderer)))
    })
}
inventory::submit! {
    ViewerLaneRendererRegistration::new(SPI_TABLE_RENDERER, || Arc::new(SpiLaneRenderer))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use egui::{Color32, Stroke};

    use logic_analyzer_viewer::ViewerLaneBadge;

    use super::*;

    #[test]
    fn spi_bit_values_use_binary_labels() {
        let renderer = SpiLaneRenderer;
        let visual = AnnotationVisual {
            label: "0x1".to_owned(),
            fill: Color32::BLACK,
            border: Stroke::new(1.0, Color32::WHITE),
        };

        assert_eq!(
            renderer
                .annotation_visual(
                    &ViewerLaneTrackId::new("bits"),
                    &ViewerLaneTheme::from_visuals(&egui::Visuals::dark(), Color32::WHITE),
                    1,
                    visual,
                )
                .label,
            "1"
        );
    }

    #[test]
    fn detail_outputs_form_one_group_per_spi_direction() {
        assert!(spi_output_presentation(0).is_none());
        assert!(spi_output_presentation(1).is_none());
        let mosi_bits = spi_output_presentation(2).unwrap();
        let mosi_data = spi_output_presentation(3).unwrap();
        assert_eq!(mosi_bits.group_key, "mosi");
        assert_eq!(mosi_data.track_key, "data");
        assert_eq!(mosi_bits.relative_height, 1.0);
        assert_eq!(mosi_data.relative_height, 1.0);
        assert_eq!(spi_output_presentation(4).unwrap().group_key, "miso");
        assert_eq!(spi_output_presentation(5).unwrap().track_key, "data");
    }

    #[test]
    fn bits_and_data_each_use_one_standard_lane_height() {
        let renderer: Arc<dyn ViewerLaneRenderer> = Arc::new(SpiLaneRenderer);
        let group = ViewerLaneGroup {
            id: logic_analyzer_viewer::ViewerLaneGroupId::new("spi"),
            label: "SPI".to_owned(),
            badge: ViewerLaneBadge::new("O", Color32::WHITE),
            tracks: vec![
                logic_analyzer_viewer::ViewerLaneTrack::new(
                    "bits",
                    DerivedLaneId::new("bits"),
                    1.0,
                ),
                logic_analyzer_viewer::ViewerLaneTrack::new(
                    "data",
                    DerivedLaneId::new("data"),
                    1.0,
                ),
            ],
            renderer: Arc::clone(&renderer),
        };

        assert_eq!(renderer.row_height(&group, 30.0), 60.0);
        let rects = group.track_rects(0.0, 60.0);
        assert_eq!(rects[0].2, 30.0);
        assert_eq!(rects[1].2, 30.0);
    }

    #[test]
    fn transaction_packet_shows_both_directions_and_completion() {
        let packet = ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 10,
            end_time_ns: 20,
            protocol_id: SPI_TRANSACTION_PROTOCOL_ID.to_owned(),
            value: ProtocolValue::Mapping(BTreeMap::from([
                ("complete".to_owned(), ProtocolValue::Bool(false)),
                (
                    "diagnostics".to_owned(),
                    ProtocolValue::List(vec![ProtocolValue::String(
                        "incomplete word: 5/8 bits".to_owned(),
                    )]),
                ),
                ("word_bits".to_owned(), ProtocolValue::Integer(8)),
                (
                    "mosi".to_owned(),
                    ProtocolValue::List(vec![
                        ProtocolValue::Integer(0x9a),
                        ProtocolValue::Integer(0xbc),
                    ]),
                ),
                (
                    "miso".to_owned(),
                    ProtocolValue::List(vec![
                        ProtocolValue::Integer(0x11),
                        ProtocolValue::Integer(0x22),
                    ]),
                ),
            ])),
        };

        assert_eq!(
            spi_transaction_label(&packet),
            "MOSI [9A BC] · MISO [11 22] · Incomplete (incomplete word: 5/8 bits)"
        );
        assert_eq!(spi_table_column(6).unwrap().source_key, "transactions");
    }
}
