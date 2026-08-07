use logic_analyzer_protocol_decoders::types::ProtocolPacket;

const MAX_LABEL_CHARS: usize = 256;

/// A protocol owner's bounded display projection for one packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolPacketDisplay {
    label: String,
    visible: bool,
    marker: bool,
}

impl ProtocolPacketDisplay {
    /// Creates a visible duration-bearing packet display projection.
    pub(crate) fn new(label: impl Into<String>) -> Self {
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

    /// Presents an instantaneous protocol event as a labeled marker.
    pub(crate) fn marker(label: impl Into<String>) -> Self {
        let mut display = Self::new(label);
        display.marker = true;
        display
    }

    /// Omits a packet span from the default viewer presentation.
    pub(crate) fn hidden() -> Self {
        Self {
            label: String::new(),
            visible: false,
            marker: false,
        }
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn is_marker(&self) -> bool {
        self.marker
    }
}

/// Compile-time packet formatter keyed by the packet's stable protocol ID.
pub(crate) struct ProtocolPacketPresentationRegistration {
    protocol_id: &'static str,
    display: fn(&ProtocolPacket) -> ProtocolPacketDisplay,
}

impl ProtocolPacketPresentationRegistration {
    /// Registers a formatter for packets with one stable protocol ID.
    ///
    /// # Parameters
    /// - `protocol_id`: Stable protocol identity accepted by the formatter.
    /// - `display`: Protocol-owned projection from a packet to display data.
    pub(crate) const fn new(
        protocol_id: &'static str,
        display: fn(&ProtocolPacket) -> ProtocolPacketDisplay,
    ) -> Self {
        Self {
            protocol_id,
            display,
        }
    }

    fn display(&self, packet: &ProtocolPacket) -> ProtocolPacketDisplay {
        (self.display)(packet)
    }
}

/// Resolves a packet through the unique formatter registered for its protocol ID.
///
/// Missing or ambiguous registrations return `None`, allowing the payload owner to use its
/// protocol-neutral fallback without depending on inventory iteration order.
///
/// # Parameters
/// - `packet`: Generic packet whose protocol-specific display projection is needed.
pub(crate) fn protocol_packet_display(packet: &ProtocolPacket) -> Option<ProtocolPacketDisplay> {
    let mut registrations = inventory::iter::<ProtocolPacketPresentationRegistration>
        .into_iter()
        .filter(|registration| registration.protocol_id == packet.protocol_id);
    let registration = registrations.next()?;
    if registrations.next().is_some() {
        return None;
    }
    Some(registration.display(packet))
}

pub(crate) fn protocol_packet_fallback_label(packet: &ProtocolPacket) -> String {
    use logic_analyzer_protocol_decoders::types::ProtocolValue;

    let value = match &packet.value {
        ProtocolValue::Null => "null".to_owned(),
        ProtocolValue::Bool(value) => value.to_string(),
        ProtocolValue::Integer(value) => value.to_string(),
        ProtocolValue::Float(value) => value.to_string(),
        ProtocolValue::String(value) => value.clone(),
        ProtocolValue::Bytes(value) => format!("{} bytes", value.len()),
        ProtocolValue::List(value) => format!("list[{}]", value.len()),
        ProtocolValue::Tuple(value) => format!("tuple[{}]", value.len()),
        ProtocolValue::Mapping(value) => format!("map[{}]", value.len()),
    };
    format!("{} · {value}", packet.protocol_id)
}

inventory::collect!(ProtocolPacketPresentationRegistration);

#[cfg(test)]
mod protocol_packet_presentation_tests {
    use logic_analyzer_protocol_decoders::types::ProtocolValue;

    use super::*;

    fn display(packet: &ProtocolPacket) -> ProtocolPacketDisplay {
        ProtocolPacketDisplay::new(format!("packet:{}", packet.protocol_id))
    }

    fn duplicate_display(_packet: &ProtocolPacket) -> ProtocolPacketDisplay {
        ProtocolPacketDisplay::new("ambiguous")
    }

    inventory::submit! {
        ProtocolPacketPresentationRegistration::new(
            "org.logicconduit.graph-registry-test.packet/v1",
            display,
        )
    }

    inventory::submit! {
        ProtocolPacketPresentationRegistration::new(
            "org.logicconduit.graph-registry-test.duplicate/v1",
            display,
        )
    }

    inventory::submit! {
        ProtocolPacketPresentationRegistration::new(
            "org.logicconduit.graph-registry-test.duplicate/v1",
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
            protocol_packet_display(&packet("org.logicconduit.graph-registry-test.packet/v1"))
                .unwrap();
        assert_eq!(
            display.label(),
            "packet:org.logicconduit.graph-registry-test.packet/v1"
        );
        assert!(protocol_packet_display(&packet("org.example.unknown/v1")).is_none());
    }

    #[test]
    fn duplicate_protocol_formatters_are_ambiguous() {
        assert!(
            protocol_packet_display(&packet("org.logicconduit.graph-registry-test.duplicate/v1"))
                .is_none()
        );
    }

    #[test]
    fn display_labels_are_bounded_at_the_payload_boundary() {
        let display = ProtocolPacketDisplay::new("x".repeat(MAX_LABEL_CHARS + 10));

        assert_eq!(display.label().chars().count(), MAX_LABEL_CHARS);
        assert!(display.label().ends_with('…'));
    }

    #[test]
    fn hidden_and_marker_packets_are_explicit_display_choices() {
        let hidden = ProtocolPacketDisplay::hidden();
        assert!(!hidden.is_visible());
        assert!(hidden.label().is_empty());

        let marker = ProtocolPacketDisplay::marker("ACK");
        assert!(marker.is_visible());
        assert!(marker.is_marker());
        assert_eq!(marker.label(), "ACK");
    }
}
