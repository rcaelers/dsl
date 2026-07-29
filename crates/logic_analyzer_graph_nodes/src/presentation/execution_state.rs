use serde_json::Value;

pub(crate) fn without_display_format(state: &Value) -> Value {
    let mut execution = state.clone();
    if let Some(fields) = execution.as_object_mut() {
        fields.remove("display_format");
    }
    execution
}

#[cfg(test)]
mod execution_state_tests {
    use super::*;

    #[test]
    fn display_format_is_excluded_but_decoder_settings_remain() {
        let state = serde_json::json!({
            "display_format": { "value": "Hex" },
            "word_size": { "value": 8 }
        });

        assert_eq!(
            without_display_format(&state),
            serde_json::json!({ "word_size": { "value": 8 } })
        );
    }
}
