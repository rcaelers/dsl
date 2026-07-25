//! Built-in collected-payload inventory submissions.

use std::sync::Arc;

use logic_analyzer_graph_api::node::CollectedPayloadRegistration;
use logic_analyzer_graph_api::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, NodeBuildContext, ResolvedInput,
};
use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerLaneRendererRegistration};
use signal_processing::{CollectedLaneRequest, CollectedWordLaneOptions, LiveStoreConfig};

use super::presentation::{
    DigitalSnapshotRenderer, NumberSnapshotRenderer, TextSnapshotRenderer, TriggerSnapshotRenderer,
    WordSnapshotRenderer,
};

fn digital_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("S", [95, 175, 95]),
        DIGITAL_RENDERER,
    )
}

fn word_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("W", [215, 140, 60]),
        WORD_RENDERER,
    )
}

fn word_request(
    request: CollectedLaneRequest,
    member: usize,
    input: &ResolvedInput,
    ctx: &dyn NodeBuildContext,
) -> CollectedLaneRequest {
    let store_config = if let Some(persistent) = ctx.derived_word_cache(member) {
        LiveStoreConfig {
            directory: persistent.directory.clone(),
            persistence: Some(persistent.clone()),
            ..LiveStoreConfig::default()
        }
    } else {
        LiveStoreConfig::default()
    };
    request.with_options(CollectedWordLaneOptions::new(
        store_config,
        input.word_display_format.clone(),
    ))
}

fn trigger_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("T", [230, 190, 80]),
        TRIGGER_RENDERER,
    )
}

fn number_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("N", [95, 145, 210]),
        NUMBER_RENDERER,
    )
}

fn text_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("TXT", [215, 150, 170]),
        TEXT_RENDERER,
    )
}

const DIGITAL_RENDERER: &str = "org.logicconduit.renderer.digital/v1";
const WORD_RENDERER: &str = "org.logicconduit.renderer.word/v1";
const TRIGGER_RENDERER: &str = "org.logicconduit.renderer.trigger/v1";
const NUMBER_RENDERER: &str = "org.logicconduit.renderer.number/v1";
const TEXT_RENDERER: &str = "org.logicconduit.renderer.text/v1";

inventory::submit! { ViewerLaneRendererRegistration::new(DIGITAL_RENDERER, || Arc::new(DigitalSnapshotRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(WORD_RENDERER, || Arc::new(WordSnapshotRenderer::new(Arc::new(DefaultViewerLaneRenderer)))) }
inventory::submit! { ViewerLaneRendererRegistration::new(TRIGGER_RENDERER, || Arc::new(TriggerSnapshotRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(NUMBER_RENDERER, || Arc::new(NumberSnapshotRenderer)) }
inventory::submit! { ViewerLaneRendererRegistration::new(TEXT_RENDERER, || Arc::new(TextSnapshotRenderer)) }

inventory::submit! {
    CollectedPayloadRegistration::subscribable::<signal_processing::Sample>(
        "org.logicconduit.digital-sample/v1",
        signal_processing::digital_payload_adapter,
        digital_presentation,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable_with_request_configurator::<signal_processing::Word>(
        "org.logicconduit.word/v1",
        signal_processing::word_payload_adapter,
        word_presentation,
        word_request,
        true,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable::<signal_processing::Trigger>(
        "org.logicconduit.trigger/v1",
        signal_processing::trigger_payload_adapter,
        trigger_presentation,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable::<signal_processing::NumberSample>(
        "org.logicconduit.number-sample/v1",
        signal_processing::number_payload_adapter,
        number_presentation,
    )
}

inventory::submit! {
    CollectedPayloadRegistration::subscribable::<signal_processing::TextSample>(
        "org.logicconduit.text-sample/v1",
        signal_processing::text_payload_adapter,
        text_presentation,
    )
}
