use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokAnnotation, SigrokBinary, SigrokGeneratedLogic, SigrokLaneSnapshot, SigrokMetadata,
};
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneRenderer, ViewerLaneTrack, draw_span_snapshot,
};
use signal_processing::{OpaqueCollectedLaneSnapshot, ProtocolPacket};

macro_rules! span_renderer {
    ($renderer:ident, $payload:ty) => {
        pub(crate) struct $renderer;

        impl ViewerLaneRenderer for $renderer {
            fn draw_opaque_lane(
                &self,
                _track: &ViewerLaneTrack,
                snapshot: Option<&OpaqueCollectedLaneSnapshot>,
                context: OpaqueLaneDrawContext<'_>,
            ) -> bool {
                let Some(snapshot) =
                    snapshot.and_then(|snapshot| snapshot.value::<SigrokLaneSnapshot<$payload>>())
                else {
                    return false;
                };
                let values = snapshot
                    .entries()
                    .iter()
                    .map(|entry| (entry.start_time_ns, entry.end_time_ns, entry.display_text()))
                    .collect::<Vec<_>>();
                draw_span_snapshot(&context, &values, context.theme.accent);
                if !snapshot.activity_spans().is_empty() {
                    let activity = snapshot
                        .activity_spans()
                        .iter()
                        .map(|&(start, end)| (start, end, String::new()))
                        .collect::<Vec<_>>();
                    draw_span_snapshot(&context, &activity, context.theme.accent);
                }
                true
            }
        }
    };
}

span_renderer!(SigrokAnnotationRenderer, SigrokAnnotation);
span_renderer!(SigrokBinaryRenderer, SigrokBinary);
span_renderer!(SigrokGeneratedLogicRenderer, SigrokGeneratedLogic);
span_renderer!(SigrokMetadataRenderer, SigrokMetadata);
span_renderer!(ProtocolPacketRenderer, ProtocolPacket);
