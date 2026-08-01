use input_bindings::InputBindings;
use logic_analyzer_viewer::ColorProfile;

/// Portable application settings consumed by the UI.
#[derive(Clone, Debug)]
pub struct ApplicationSettings {
    viewer_color_profile: ColorProfile,
    max_recent_capture_sessions: usize,
    max_capture_storage_gib: u64,
}

impl ApplicationSettings {
    /// Decodes settings acquired by a host adapter.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let document: ApplicationSettingsDocument = serde_json::from_str(json)?;
        Ok(Self {
            viewer_color_profile: document.logic_analyzer_viewer.color_profile.into(),
            max_recent_capture_sessions: document.live_capture.max_recent_sessions,
            max_capture_storage_gib: document.live_capture.max_storage_gib,
        })
    }

    pub fn viewer_color_profile(&self) -> ColorProfile {
        self.viewer_color_profile
    }

    pub fn max_recent_capture_sessions(&self) -> usize {
        self.max_recent_capture_sessions
    }

    pub fn max_capture_storage_gib(&self) -> u64 {
        self.max_capture_storage_gib
    }
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self::from_json(include_str!("../config/application.json"))
            .expect("embedded application configuration must be valid")
    }
}

/// The default key and pointer bindings shipped with the application.
pub fn default_input_bindings() -> InputBindings {
    InputBindings::from_json(include_str!("../config/input_bindings.json"))
        .expect("embedded application input bindings must be valid")
}

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ApplicationSettingsDocument {
    logic_analyzer_viewer: LogicAnalyzerViewerSettings,
    live_capture: LiveCaptureSettings,
}

#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LogicAnalyzerViewerSettings {
    color_profile: ConfiguredColorProfile,
}

impl Default for LogicAnalyzerViewerSettings {
    fn default() -> Self {
        Self {
            color_profile: ConfiguredColorProfile::DsView,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LiveCaptureSettings {
    max_recent_sessions: usize,
    max_storage_gib: u64,
}

impl Default for LiveCaptureSettings {
    fn default() -> Self {
        Self {
            max_recent_sessions: 10,
            max_storage_gib: 20,
        }
    }
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConfiguredColorProfile {
    DsView,
    Classic,
}

impl From<ConfiguredColorProfile> for ColorProfile {
    fn from(profile: ConfiguredColorProfile) -> Self {
        match profile {
            ConfiguredColorProfile::DsView => Self::DsView,
            ConfiguredColorProfile::Classic => Self::Classic,
        }
    }
}

#[cfg(test)]
mod application_settings_tests {
    use logic_analyzer_viewer::ColorProfile;

    use super::ApplicationSettings;

    #[test]
    fn host_supplied_settings_decode_to_the_ui_model() {
        let settings = ApplicationSettings::from_json(
            r#"{
                "logic_analyzer_viewer": { "color_profile": "classic" },
                "live_capture": { "max_recent_sessions": 7, "max_storage_gib": 12 }
            }"#,
        )
        .unwrap();

        assert_eq!(settings.viewer_color_profile(), ColorProfile::Classic);
        assert_eq!(settings.max_recent_capture_sessions(), 7);
        assert_eq!(settings.max_capture_storage_gib(), 12);
    }
}
