use serde_json::Value;

use logic_analyzer_graph_capabilities::node::LiveCaptureFeatureError;
use logic_analyzer_graph_capabilities::node_support::{
    LiveCaptureEdit, parse_state, serialize_state,
};

use super::definition::U3Pro16State;

pub(crate) fn apply(
    state: &Value,
    edit: &LiveCaptureEdit,
) -> Result<Value, LiveCaptureFeatureError> {
    let mut state = parse_state::<U3Pro16State>(state)?;
    match edit {
        LiveCaptureEdit::SetSimpleTrigger {
            channel_id,
            condition,
        } => {
            let physical_channel = channel_id
                .as_str()
                .strip_prefix("u3pro16:input:")
                .and_then(|channel| channel.parse::<usize>().ok())
                .ok_or_else(|| {
                    LiveCaptureFeatureError::edit(format!("unknown U3Pro16 input {channel_id}"))
                })?;
            state
                .set_trigger_condition(physical_channel, *condition)
                .map_err(LiveCaptureFeatureError::edit)?;
        }
        LiveCaptureEdit::SetTriggerProgram { program } => {
            state
                .set_trigger_program(program.clone())
                .map_err(LiveCaptureFeatureError::edit)?;
        }
    }
    serialize_state(state).map_err(Into::into)
}
