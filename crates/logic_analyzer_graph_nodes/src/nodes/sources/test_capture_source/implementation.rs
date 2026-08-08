//! Test-capture state editing.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::LiveCaptureFeatureError;
use logic_analyzer_graph_capabilities::node_support::{
    LiveCaptureEdit, parse_state, serialize_state,
};

use super::definition::TestCaptureSourceState;

pub(crate) fn apply_live_capture_edit(
    state: &Value,
    edit: &LiveCaptureEdit,
) -> Result<Value, LiveCaptureFeatureError> {
    let mut state = parse_state::<TestCaptureSourceState>(state)?;
    match edit {
        LiveCaptureEdit::SetSimpleTrigger {
            channel_id,
            condition,
        } => {
            let channel = channel_id
                .as_str()
                .strip_prefix("demo:")
                .and_then(|channel| channel.parse::<usize>().ok())
                .ok_or_else(|| {
                    LiveCaptureFeatureError::edit(format!(
                        "unknown test capture channel {channel_id}"
                    ))
                })?;
            state
                .set_trigger_condition(channel, *condition)
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
