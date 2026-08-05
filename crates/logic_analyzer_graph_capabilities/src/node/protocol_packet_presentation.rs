//! Protocol-owned presentation of generic protocol packets.

const MAX_LABEL_CHARS: usize = 256;

/// A protocol owner's bounded display projection for one packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolPacketDisplay {
    label: String,
    visible: bool,
    marker: bool,
}

impl ProtocolPacketDisplay {
    /// Creates a visible duration-bearing packet display projection.
    ///
    /// # Parameters
    /// - `label`: Protocol-owned label, truncated to the display length limit.
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

    /// Returns the bounded user-facing packet label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns whether the viewer should render this packet.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Returns whether the packet is rendered as an instantaneous marker.
    pub fn is_marker(&self) -> bool {
        self.marker
    }
}

#[cfg(test)]
mod protocol_packet_presentation_tests {
    use super::*;

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
