use logic_analyzer_graph_api::node_support::CapturePresentation;

use super::definition::U3Pro16State;

pub(crate) fn capture_presentation(state: &U3Pro16State) -> Result<CapturePresentation, String> {
    let channels = state
        .channels
        .enabled
        .iter()
        .enumerate()
        .filter(|(_, enabled)| **enabled)
        .enumerate()
        .map(|(viewer_channel, (physical_channel, _))| {
            (viewer_channel, format!("Ch {physical_channel}"))
        })
        .collect();
    Ok(CapturePresentation::Channels(channels))
}
