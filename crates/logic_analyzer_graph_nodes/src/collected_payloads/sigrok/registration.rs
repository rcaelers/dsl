use std::sync::Arc;

use logic_analyzer_graph_api::node::CollectedPayloadRegistration;
use logic_analyzer_graph_api::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, PortKind,
};
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokAnnotation, SigrokBinary, SigrokGeneratedLogic, SigrokMetadata,
    sigrok_annotation_payload_adapter, sigrok_binary_payload_adapter,
    sigrok_generated_logic_payload_adapter, sigrok_metadata_payload_adapter,
    sigrok_protocol_packet_payload_adapter,
};
use logic_analyzer_viewer::ViewerLaneRendererRegistration;
use signal_processing::ProtocolPacket;

use super::presentation::{
    ProtocolPacketRenderer, SigrokAnnotationRenderer, SigrokBinaryRenderer,
    SigrokGeneratedLogicRenderer, SigrokMetadataRenderer,
};

fn annotation_kind() -> PortKind {
    PortKind::of_named::<SigrokAnnotation>("Sigrok Annotation")
}

fn binary_kind() -> PortKind {
    PortKind::of_named::<SigrokBinary>("Sigrok Binary")
}

fn generated_logic_kind() -> PortKind {
    PortKind::of_named::<SigrokGeneratedLogic>("Sigrok Logic")
}

fn metadata_kind() -> PortKind {
    PortKind::of_named::<SigrokMetadata>("Sigrok Metadata")
}

fn protocol_packet_kind() -> PortKind {
    PortKind::of_named::<ProtocolPacket>("Protocol Packet")
}

fn annotation_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("A", [220, 155, 65]),
        ANNOTATION_RENDERER,
    )
}

fn binary_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("BIN", [205, 125, 55]),
        BINARY_RENDERER,
    )
}

fn generated_logic_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("S", [95, 175, 95]),
        GENERATED_LOGIC_RENDERER,
    )
}

fn metadata_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("M", [95, 145, 210]),
        METADATA_RENDERER,
    )
}

fn protocol_packet_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("P", [175, 120, 205]),
        PROTOCOL_PACKET_RENDERER,
    )
}

const ANNOTATION_RENDERER: &str = "org.logicconduit.renderer.sigrok-annotation/v1";
const BINARY_RENDERER: &str = "org.logicconduit.renderer.sigrok-binary/v1";
const GENERATED_LOGIC_RENDERER: &str = "org.logicconduit.renderer.sigrok-logic/v1";
const METADATA_RENDERER: &str = "org.logicconduit.renderer.sigrok-metadata/v1";
const PROTOCOL_PACKET_RENDERER: &str = "org.logicconduit.renderer.protocol-packet/v1";

inventory::submit! { ViewerLaneRendererRegistration::new(ANNOTATION_RENDERER, || Arc::new(SigrokAnnotationRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(BINARY_RENDERER, || Arc::new(SigrokBinaryRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(GENERATED_LOGIC_RENDERER, || Arc::new(SigrokGeneratedLogicRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(METADATA_RENDERER, || Arc::new(SigrokMetadataRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(PROTOCOL_PACKET_RENDERER, || Arc::new(ProtocolPacketRenderer)) }

inventory::submit! {
    CollectedPayloadRegistration::subscribable_kind(
        "org.logicconduit.sigrok.annotation/v1",
        annotation_kind,
        sigrok_annotation_payload_adapter,
        annotation_presentation,
    )
}

#[cfg(test)]
mod registration_tests {
    use super::*;

    #[test]
    fn sigrok_payload_kinds_have_distinct_open_type_identities() {
        let kinds = [
            annotation_kind(),
            binary_kind(),
            generated_logic_kind(),
            metadata_kind(),
            protocol_packet_kind(),
        ];
        for (index, kind) in kinds.iter().enumerate() {
            assert!(kinds.iter().skip(index + 1).all(|other| kind != other));
        }
    }

    #[test]
    fn all_sigrok_payload_contracts_are_submitted_to_inventory() {
        let stable_ids = inventory::iter::<CollectedPayloadRegistration>
            .into_iter()
            .map(CollectedPayloadRegistration::stable_id)
            .collect::<std::collections::HashSet<_>>();
        for stable_id in [
            "org.logicconduit.sigrok.annotation/v1",
            "org.logicconduit.sigrok.binary/v1",
            "org.logicconduit.sigrok.generated-logic/v1",
            "org.logicconduit.sigrok.metadata/v1",
            "org.logicconduit.protocol-packet/v1",
        ] {
            assert!(stable_ids.contains(stable_id));
        }
    }
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable_kind(
        "org.logicconduit.sigrok.binary/v1",
        binary_kind,
        sigrok_binary_payload_adapter,
        binary_presentation,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable_kind(
        "org.logicconduit.sigrok.generated-logic/v1",
        generated_logic_kind,
        sigrok_generated_logic_payload_adapter,
        generated_logic_presentation,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable_kind(
        "org.logicconduit.sigrok.metadata/v1",
        metadata_kind,
        sigrok_metadata_payload_adapter,
        metadata_presentation,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable_kind(
        "org.logicconduit.protocol-packet/v1",
        protocol_packet_kind,
        sigrok_protocol_packet_payload_adapter,
        protocol_packet_presentation,
    )
}
