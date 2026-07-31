use logic_analyzer_graph_api::node_support::CapturePresentation;

use super::definition::U3Pro16State;

pub(crate) fn capture_presentation(state: &U3Pro16State) -> Result<CapturePresentation, String> {
    let names = state
        .channels
        .enabled
        .iter()
        .enumerate()
        .filter(|(_, enabled)| **enabled)
        .map(|(channel, _)| format!("Ch {channel}"));
    Ok(super::super::synthetic_presentation::capture_presentation(
        names,
    ))
}
