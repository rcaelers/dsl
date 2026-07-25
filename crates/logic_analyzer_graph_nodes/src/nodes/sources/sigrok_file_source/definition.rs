//! `Sigrok File Source` graph-node definition.

use std::path::Path;

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    FileValue, InputDef, IntValue, NodeBadge, NodeDef, NodeInstanceSchema, OutputDef, Socket,
};

use super::metadata_platform;
use crate::nodes::registry::{COLOR_SOURCES, Signal, TextOpenPath};

const DEFAULT_DEMO_CHANNELS: usize = 11;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SigrokFileSourceState {
    pub(crate) file: FileValue,
    #[serde(default)]
    pub(crate) channel_names: Vec<String>,
    #[serde(default)]
    pub(crate) metadata_path: String,
    #[serde(default, rename = "channels", skip_serializing)]
    pub(crate) legacy_channels: Option<IntValue>,
    #[serde(default)]
    pub(crate) demo_data: bool,
    #[serde(skip)]
    pub(crate) diagnostic: Option<String>,
    #[serde(skip)]
    pub(crate) compatibility_warning: Option<String>,
}

impl SigrokFileSourceState {
    pub(crate) fn channel_count(&self) -> usize {
        self.channel_names.len()
    }
}

pub(crate) struct SigrokFileSource;

impl NodeDef for SigrokFileSource {
    type State = SigrokFileSourceState;

    fn name() -> &'static str {
        "Sigrok File Source"
    }

    fn category() -> &'static str {
        "Sources"
    }

    fn color() -> Color32 {
        COLOR_SOURCES
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::control::<TextOpenPath>("File", |state| {
            &mut state.file
        })]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        Vec::new()
    }

    fn state() -> Self::State {
        SigrokFileSourceState {
            file: FileValue::with_filter("", "Select sigrok capture", "Sigrok captures", &["sr"]),
            channel_names: Vec::new(),
            metadata_path: String::new(),
            legacy_channels: None,
            demo_data: false,
            diagnostic: None,
            compatibility_warning: None,
        }
    }

    fn instance_schema(state: &Self::State) -> NodeInstanceSchema<Self::State> {
        let channel_names = if state.channel_names.is_empty() {
            state
                .legacy_channels
                .as_ref()
                .map(|legacy| generic_channel_names(legacy.value.clamp(1, 32) as usize))
                .unwrap_or_default()
        } else {
            state.channel_names.clone()
        };
        let outputs = channel_names
            .into_iter()
            .enumerate()
            .map(|(channel, name)| {
                OutputDef::new::<Signal>(name).stable_id(format!("Ch {channel}"))
            })
            .collect();
        NodeInstanceSchema::new(Self::inputs(), outputs).panels(Self::panels())
    }

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        state.diagnostic = None;
        if state.channel_names.is_empty()
            && let Some(legacy) = state.legacy_channels.take()
        {
            state.channel_names = generic_channel_names(legacy.value.clamp(1, 32) as usize);
            state.compatibility_warning = Some(
                "Upgraded the saved Sigrok source to derive its channels from file metadata".into(),
            );
        }

        if state.demo_data {
            if state.channel_names.is_empty() {
                state.channel_names = generic_channel_names(DEFAULT_DEMO_CHANNELS);
            }
            state.metadata_path.clone_from(&state.file.value);
            return;
        }

        let path_changed = state.metadata_path != state.file.value;
        if state.file.value.trim().is_empty() {
            state.channel_names.clear();
            state.metadata_path.clear();
            return;
        }
        if !path_changed && !state.channel_names.is_empty() {
            return;
        }

        match metadata_platform::channel_names(Path::new(&state.file.value)) {
            Ok(Some(names)) => {
                state.channel_names = names;
                state.metadata_path.clone_from(&state.file.value);
            }
            Ok(None) if !state.channel_names.is_empty() => {
                state.metadata_path.clone_from(&state.file.value);
            }
            Ok(None) => {
                state.diagnostic = Some("Channel metadata is unavailable on this platform".into());
            }
            Err(error) => {
                if path_changed {
                    state.channel_names.clear();
                }
                state.diagnostic = Some(format!("Could not inspect Sigrok file: {error}"));
            }
        }
    }

    fn badge(state: &Self::State) -> Option<NodeBadge> {
        state
            .diagnostic
            .as_ref()
            .map(NodeBadge::error)
            .or_else(|| state.compatibility_warning.as_ref().map(NodeBadge::warning))
    }
}

fn generic_channel_names(count: usize) -> Vec<String> {
    (0..count).map(|channel| format!("Ch {channel}")).collect()
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn legacy_channel_count_preserves_sockets_then_disappears_as_a_control() {
        let mut state = SigrokFileSource::state();
        state.demo_data = true;
        state.legacy_channels = Some(IntValue::new(5, 1, 32));

        let initial = SigrokFileSource::instance_schema(&state);
        assert_eq!(initial.inputs.len(), 1);
        assert_eq!(initial.outputs.len(), 5);

        SigrokFileSource::on_update(&mut state, &mut [], &mut []);
        let migrated = SigrokFileSource::instance_schema(&state);
        assert_eq!(migrated.inputs.len(), 1);
        assert_eq!(migrated.outputs.len(), 5);
        assert!(state.legacy_channels.is_none());
        assert!(state.compatibility_warning.is_some());
    }
}
