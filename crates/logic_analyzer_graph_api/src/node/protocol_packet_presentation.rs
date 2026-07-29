//! Protocol-owned presentation of generic protocol packets.

use signal_processing::ProtocolPacket;

const MAX_LABEL_CHARS: usize = 256;

/// A protocol owner's bounded display projection for one packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolPacketDisplay {
    label: String,
    visible: bool,
    marker: bool,
}

impl ProtocolPacketDisplay {
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        if label.chars().count() <= MAX_LABEL_CHARS {
            return Self {
                label,
                visible: true,
                marker: false,
            };
        }
        let mut label = label.chars().take(MAX_LABEL_CHARS - 1).collect::<String>();
        label.push('…');
        Self {
            label,
            visible: true,
            marker: false,
        }
    }

    /// Presents an instantaneous protocol event as a labeled marker rather
    /// than as a duration-bearing packet span.
    pub fn marker(label: impl Into<String>) -> Self {
        let mut display = Self::new(label);
        display.marker = true;
        display
    }

    /// Keeps a protocol packet available to downstream consumers while
    /// omitting its span from the default viewer presentation.
    pub fn hidden() -> Self {
        Self {
            label: String::new(),
            visible: false,
            marker: false,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn is_marker(&self) -> bool {
        self.marker
    }
}

/// Compile-time packet formatter keyed by the packet's stable protocol ID.
pub struct ProtocolPacketPresentationRegistration {
    protocol_id: &'static str,
    display: fn(&ProtocolPacket) -> ProtocolPacketDisplay,
}

impl ProtocolPacketPresentationRegistration {
    pub const fn new(
        protocol_id: &'static str,
        display: fn(&ProtocolPacket) -> ProtocolPacketDisplay,
    ) -> Self {
        Self {
            protocol_id,
            display,
        }
    }

    pub const fn protocol_id(&self) -> &'static str {
        self.protocol_id
    }

    fn display(&self, packet: &ProtocolPacket) -> ProtocolPacketDisplay {
        (self.display)(packet)
    }
}

/// Resolves a packet through the formatter registered for its protocol ID.
///
/// Missing or ambiguous registrations return `None`, allowing the payload
/// owner to use its protocol-neutral fallback without depending on inventory
/// iteration order.
pub fn protocol_packet_display(packet: &ProtocolPacket) -> Option<ProtocolPacketDisplay> {
    let mut registrations = inventory::iter::<ProtocolPacketPresentationRegistration>
        .into_iter()
        .filter(|registration| registration.protocol_id == packet.protocol_id);
    let registration = registrations.next()?;
    if registrations.next().is_some() {
        return None;
    }
    Some(registration.display(packet))
}

inventory::collect!(ProtocolPacketPresentationRegistration);

#[cfg(test)]
mod protocol_packet_presentation_tests {
    use signal_processing::ProtocolValue;

    use super::*;

    fn display(packet: &ProtocolPacket) -> ProtocolPacketDisplay {
        ProtocolPacketDisplay::new(format!("packet:{}", packet.protocol_id))
    }

    fn duplicate_display(_packet: &ProtocolPacket) -> ProtocolPacketDisplay {
        ProtocolPacketDisplay::new("ambiguous")
    }

    inventory::submit! {
        ProtocolPacketPresentationRegistration::new(
            "org.logicconduit.graph-api-test.packet/v1",
            display,
        )
    }

    inventory::submit! {
        ProtocolPacketPresentationRegistration::new(
            "org.logicconduit.graph-api-test.duplicate/v1",
            display,
        )
    }

    inventory::submit! {
        ProtocolPacketPresentationRegistration::new(
            "org.logicconduit.graph-api-test.duplicate/v1",
            duplicate_display,
        )
    }

    fn packet(protocol_id: &str) -> ProtocolPacket {
        ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 0,
            end_time_ns: 1,
            protocol_id: protocol_id.to_owned(),
            value: ProtocolValue::Null,
        }
    }

    #[test]
    fn formatter_is_selected_by_exact_protocol_identity() {
        let display =
            protocol_packet_display(&packet("org.logicconduit.graph-api-test.packet/v1")).unwrap();

        assert_eq!(
            display.label(),
            "packet:org.logicconduit.graph-api-test.packet/v1"
        );
        assert!(protocol_packet_display(&packet("org.example.unknown/v1")).is_none());
    }

    #[test]
    fn duplicate_protocol_formatters_are_ambiguous() {
        assert!(
            protocol_packet_display(&packet("org.logicconduit.graph-api-test.duplicate/v1"))
                .is_none()
        );
    }

    #[test]
    fn display_labels_are_bounded_at_the_plugin_boundary() {
        let display = ProtocolPacketDisplay::new("x".repeat(MAX_LABEL_CHARS + 10));

        assert_eq!(display.label().chars().count(), MAX_LABEL_CHARS);
        assert!(display.label().ends_with('…'));
    }

    #[test]
    fn hidden_packets_remain_an_explicit_protocol_presentation_choice() {
        let display = ProtocolPacketDisplay::hidden();

        assert!(!display.is_visible());
        assert!(display.label().is_empty());
    }

    #[test]
    fn marker_packets_are_distinct_from_duration_spans() {
        let display = ProtocolPacketDisplay::marker("ACK");

        assert!(display.is_visible());
        assert!(display.is_marker());
        assert_eq!(display.label(), "ACK");
    }
}
