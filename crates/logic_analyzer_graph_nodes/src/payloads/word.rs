use std::sync::Arc;

use logic_analyzer_graph_api::node::PayloadRegistration;
use logic_analyzer_graph_api::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, NodeBuildContext, ResolvedInput,
};
use logic_analyzer_viewer::{
    AnnotationVisual, DefaultViewerLaneRenderer, DerivedLaneId, OpaqueLaneDrawContext,
    ViewerLaneGroup, ViewerLaneRenderer, ViewerLaneRendererRegistration, ViewerLaneTheme,
    ViewerLaneTrack, ViewerLaneTrackId, default_annotation_visual, draw_annotation_presence,
    draw_annotation_snapshot,
};
use signal_processing::{
    CollectedLaneRequest, CollectedWordLaneOptions, LiveStoreConfig, OpaqueCollectedLaneSnapshot,
    WordLaneSnapshot, WordPayload,
};

const RENDERER: &str = "org.logicconduit.renderer.word/v1";

pub(crate) struct WordSnapshotRenderer {
    semantics: Arc<dyn ViewerLaneRenderer>,
}

impl WordSnapshotRenderer {
    pub(crate) fn new(semantics: Arc<dyn ViewerLaneRenderer>) -> Self {
        Self { semantics }
    }
}

impl ViewerLaneRenderer for WordSnapshotRenderer {
    fn row_height(&self, group: &ViewerLaneGroup, base_height: f32) -> f32 {
        self.semantics.row_height(group, base_height)
    }

    fn annotation_visual(
        &self,
        track: &ViewerLaneTrackId,
        theme: &ViewerLaneTheme,
        value: u64,
        default: AnnotationVisual,
    ) -> AnnotationVisual {
        self.semantics
            .annotation_visual(track, theme, value, default)
    }

    fn draw_opaque_lane(
        &self,
        track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        let Some(snapshot) = snapshot.and_then(|snapshot| snapshot.value::<WordLaneSnapshot>())
        else {
            return false;
        };
        match snapshot.as_ref() {
            WordLaneSnapshot::Exact {
                annotations,
                last_timestamp_ns,
                display_format,
            } => {
                draw_annotation_snapshot(&context, annotations, *last_timestamp_ns, |annotation| {
                    let default = default_annotation_visual(
                        annotation.value,
                        display_format.as_deref(),
                        &context.theme,
                    );
                    let mut visual = self.semantics.annotation_visual(
                        &track.id,
                        &context.theme,
                        annotation.value,
                        default,
                    );
                    if let Some(payload) = &annotation.payload {
                        visual.label = match payload {
                            WordPayload::Bytes(bytes) => bytes
                                .iter()
                                .map(|byte| format!("{byte:02X}"))
                                .collect::<Vec<_>>()
                                .join(" "),
                            WordPayload::Text(text) => text.to_string(),
                        };
                    }
                    visual
                })
            }
            WordLaneSnapshot::Presence(buckets) => draw_annotation_presence(
                &context,
                buckets
                    .iter()
                    .map(|bucket| (bucket.start_ns, bucket.end_ns, bucket.word_count)),
            ),
            WordLaneSnapshot::Activity => {
                let top = context.top + context.height * 0.12;
                let bottom = context.top + context.height * 0.88;
                context.painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::Pos2::new(context.wave_rect.left(), top),
                        egui::Pos2::new(context.wave_rect.right(), bottom),
                    ),
                    0.0,
                    context.theme.accent,
                );
            }
            WordLaneSnapshot::Error => return false,
        }
        true
    }

    fn snap_lanes(&self, group: &ViewerLaneGroup, pointer_fraction: f32) -> Vec<DerivedLaneId> {
        self.semantics.snap_lanes(group, pointer_fraction)
    }
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(LaneBadgeDescriptor::new("W", [215, 140, 60]), RENDERER)
}

fn request(
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
        store_config
            .with_work_executor(ctx.work_executor())
            .with_artifact_repository(ctx.artifact_repository()),
        input.word_display_format.clone(),
    ))
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(RENDERER, || {
        Arc::new(WordSnapshotRenderer::new(Arc::new(DefaultViewerLaneRenderer)))
    })
}

inventory::submit! {
    PayloadRegistration::subscribable_with_request_configurator::<signal_processing::Word>(
        "org.logicconduit.word/v1",
        signal_processing::word_payload_adapter,
        presentation,
        request,
        true,
    )
}
