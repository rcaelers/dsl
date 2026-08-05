use logic_analyzer_graph_capabilities::node::ProtocolPacketDisplay;
use signal_derived::ProtocolPacket;

/// Compile-time packet formatter keyed by the packet's stable protocol ID.
pub struct ProtocolPacketPresentationRegistration {
    protocol_id: &'static str,
    display: fn(&ProtocolPacket) -> ProtocolPacketDisplay,
}

impl ProtocolPacketPresentationRegistration {
    /// Registers a formatter for packets with one stable protocol ID.
    ///
    /// # Parameters
    /// - `protocol_id`: Stable protocol identity accepted by the formatter.
    /// - `display`: Protocol-owned projection from a packet to display data.
    pub const fn new(
        protocol_id: &'static str,
        display: fn(&ProtocolPacket) -> ProtocolPacketDisplay,
    ) -> Self {
        Self {
            protocol_id,
            display,
        }
    }

    /// Returns the stable protocol identity claimed by this formatter.
    pub const fn protocol_id(&self) -> &'static str {
        self.protocol_id
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
    use signal_derived::ProtocolValue;

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
}
