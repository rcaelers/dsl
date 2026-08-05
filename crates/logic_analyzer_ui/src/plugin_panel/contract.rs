//! Public contracts implemented by compile-time plugin panels.

use signal_derived::OpaqueCollectedLane;

/// Read-only application data exposed while a plugin panel is drawn.
pub struct PluginPanelContext<'a> {
    lanes: &'a [OpaqueCollectedLane],
}

impl<'a> PluginPanelContext<'a> {
    pub(crate) fn new(lanes: &'a [OpaqueCollectedLane]) -> Self {
        Self { lanes }
    }

    /// Returns the read-only collected lanes available to this draw call.
    pub fn collected_lanes(&self) -> &'a [OpaqueCollectedLane] {
        self.lanes
    }
}

/// One independently persisted panel instance.
/// Persistable panel implementation contributed by a plugin feature.
///
/// Plugin panels own their state and rendering but receive application data only through
/// [`PluginPanelContext`].
pub trait PluginPanel: Send {
    /// Draws the panel for the current UI frame.
    ///
    /// # Parameters
    /// - `ui`: Egui UI allocated to this panel instance.
    /// - `context`: Read-only application data available to the panel.
    fn show(&mut self, ui: &mut egui::Ui, context: PluginPanelContext<'_>);

    /// Serializes the panel's owner-managed persistent state.
    ///
    /// The default indicates that the panel has no saved state.
    fn save_state(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Restores owner-managed state previously returned by [`Self::save_state`].
    ///
    /// # Parameters
    /// - `_state`: Persisted JSON value for this panel instance.
    ///
    /// The default accepts no state and leaves the panel unchanged.
    fn restore_state(&mut self, _state: serde_json::Value) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PluginPanelIcon {
    #[default]
    Panel,
    Image,
    List,
    Table,
}

/// Runtime registration metadata built from an inventory submission.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PluginPanelDescriptor {
    pub(crate) stable_id: String,
    pub(crate) title: String,
    pub(crate) icon: PluginPanelIcon,
    pub(crate) minimum_width: f32,
    pub(crate) minimum_height: f32,
    pub(crate) singleton: bool,
}

impl PluginPanelDescriptor {
    pub(crate) fn new(stable_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            stable_id: stable_id.into(),
            title: title.into(),
            icon: PluginPanelIcon::Panel,
            minimum_width: 180.0,
            minimum_height: 120.0,
            singleton: false,
        }
    }

    pub(crate) fn icon(mut self, icon: PluginPanelIcon) -> Self {
        self.icon = icon;
        self
    }

    pub(crate) fn minimum_size(mut self, width: f32, height: f32) -> Self {
        self.minimum_width = width;
        self.minimum_height = height;
        self
    }

    pub(crate) fn singleton(mut self) -> Self {
        self.singleton = true;
        self
    }
}
