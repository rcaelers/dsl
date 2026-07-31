use serde_json::Value;

use logic_analyzer_graph_api::node_support::{LiveCaptureEdit, parse_state};

use super::definition::U3Pro16State;

pub(crate) fn apply(state: &Value, edit: &LiveCaptureEdit) -> Result<Value, String> {
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
                .ok_or_else(|| format!("unknown U3Pro16 input {channel_id}"))?;
            state.set_trigger_condition(physical_channel, *condition)?;
        }
        LiveCaptureEdit::SetTriggerProgram { program } => {
            state.set_trigger_program(program.clone())?;
        }
    }
    serde_json::to_value(state).map_err(|error| error.to_string())
}
