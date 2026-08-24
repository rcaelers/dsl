//! Runtime instances for registered plugin panel kinds.

use std::collections::HashMap;
use std::sync::Arc;

use signal_derived::{DerivedLanes, OpaqueCollectedLane};

use super::contract::{PluginPanel, PluginPanelContext, PluginPanelDescriptor, PluginPanelIcon};
use super::error::{
    PluginPanelRegistrationError, PluginPanelRestoreError, validate_plugin_panel_definition,
};

type PluginPanelFactory = Arc<dyn Fn() -> Box<dyn PluginPanel> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct PluginPanelDefinition {
    pub(crate) stable_id: String,
    pub(crate) title: String,
    pub(crate) icon: PluginPanelIcon,
    pub(crate) minimum_width: f32,
    pub(crate) minimum_height: f32,
    pub(crate) singleton: bool,
}

struct RegisteredPluginPanel {
    definition: PluginPanelDefinition,
    factory: PluginPanelFactory,
}

#[derive(Default)]
pub(crate) struct PluginPanelRegistry {
    panels: Vec<RegisteredPluginPanel>,
}

impl PluginPanelRegistry {
    pub(crate) fn standard() -> Result<Self, PluginPanelRegistrationError> {
        let mut registry = Self::default();
        for registration in super::registration::ui_panel_registrations()? {
            registration.apply_to(&mut registry)?;
        }
        Ok(registry)
    }

    pub(crate) fn register<P>(
        &mut self,
        descriptor: PluginPanelDescriptor,
    ) -> Result<(), PluginPanelRegistrationError>
    where
        P: PluginPanel + Default + 'static,
    {
        validate_plugin_panel_definition(&descriptor.stable_id, &descriptor.title)?;
        if self
            .panels
            .iter()
            .any(|panel| panel.definition.stable_id == descriptor.stable_id)
        {
            return Err(PluginPanelRegistrationError::DuplicateStableId {
                stable_id: descriptor.stable_id,
            });
        }
        self.panels.push(RegisteredPluginPanel {
            definition: PluginPanelDefinition {
                stable_id: descriptor.stable_id,
                title: descriptor.title,
                icon: descriptor.icon,
                minimum_width: descriptor.minimum_width,
                minimum_height: descriptor.minimum_height,
                singleton: descriptor.singleton,
            },
            factory: Arc::new(|| Box::<P>::default()),
        });
        Ok(())
    }
}

#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub(crate) struct PluginPanelsState {
    panels: HashMap<String, HashMap<String, serde_json::Value>>,
}

pub(crate) struct PluginPanels {
    registry: PluginPanelRegistry,
    instances: HashMap<(String, String), Box<dyn PluginPanel>>,
    restored: PluginPanelsState,
    lanes: DerivedLanes,
}

impl PluginPanels {
    pub(crate) fn new(registry: PluginPanelRegistry) -> Self {
        Self {
            registry,
            instances: HashMap::new(),
            restored: PluginPanelsState::default(),
            lanes: DerivedLanes::new(),
        }
    }

    pub(crate) fn definitions(&self) -> Vec<PluginPanelDefinition> {
        self.registry
            .panels
            .iter()
            .map(|panel| panel.definition.clone())
            .collect()
    }

    pub(crate) fn set_run_data(&mut self, lanes: DerivedLanes) {
        self.lanes = lanes;
    }

    pub(crate) fn restore_state(&mut self, state: PluginPanelsState) {
        self.instances.clear();
        self.restored = state;
    }

    pub(crate) fn reset_state(&mut self) {
        self.restore_state(PluginPanelsState::default());
    }

    pub(crate) fn state(&self) -> PluginPanelsState {
        let mut state = self.restored.clone();
        for ((content_id, panel_id), panel) in &self.instances {
            state
                .panels
                .entry(content_id.clone())
                .or_default()
                .insert(panel_id.clone(), panel.save_state());
        }
        state
    }

    pub(crate) fn show(
        &mut self,
        content_id: &str,
        panel_id: &str,
        ui: &mut egui::Ui,
    ) -> Option<PluginPanelRestoreError> {
        let registered = self
            .registry
            .panels
            .iter()
            .find(|panel| panel.definition.stable_id == content_id)?;
        let key = (content_id.to_owned(), panel_id.to_owned());
        let mut restore_warning = None;
        let panel = self.instances.entry(key).or_insert_with(|| {
            let mut panel = (registered.factory)();
            if let Some(state) = self
                .restored
                .panels
                .get(content_id)
                .and_then(|panels| panels.get(panel_id))
                .cloned()
                && let Err(error) = panel.restore_state(state)
            {
                restore_warning = Some(PluginPanelRestoreError::new(
                    &registered.definition.title,
                    error,
                ));
            }
            panel
        });
        let lanes: Vec<OpaqueCollectedLane> = self.lanes.opaque_lanes();
        panel.show(ui, PluginPanelContext::new(&lanes));
        restore_warning
    }
}

#[cfg(test)]
mod registry_tests {
    use super::super::error::PluginPanelStateError;
    use super::*;

    #[derive(Default)]
    struct TestPanel;

    impl PluginPanel for TestPanel {
        fn show(&mut self, _ui: &mut egui::Ui, _context: PluginPanelContext<'_>) {}
    }

    #[derive(Default)]
    struct RejectingStatePanel;

    impl PluginPanel for RejectingStatePanel {
        fn show(&mut self, _ui: &mut egui::Ui, _context: PluginPanelContext<'_>) {}

        fn restore_state(
            &mut self,
            _state: serde_json::Value,
        ) -> Result<(), PluginPanelStateError> {
            Err(PluginPanelStateError::message(
                "unsupported state version 9",
            ))
        }
    }

    #[test]
    fn registered_panel_is_discoverable_without_application_dispatch_changes() {
        let mut registry = PluginPanelRegistry::default();
        registry
            .register::<TestPanel>(
                PluginPanelDescriptor::new("org.example.camera/v1", "Camera")
                    .icon(PluginPanelIcon::Image),
            )
            .unwrap();
        let panels = PluginPanels::new(registry);
        let definitions = panels.definitions();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].stable_id, "org.example.camera/v1");
        assert_eq!(definitions[0].icon, PluginPanelIcon::Image);
    }

    #[test]
    fn duplicate_stable_panel_identity_is_rejected() {
        let mut registry = PluginPanelRegistry::default();
        let descriptor = PluginPanelDescriptor::new("org.example.camera/v1", "Camera");
        registry.register::<TestPanel>(descriptor.clone()).unwrap();

        assert_eq!(
            registry.register::<TestPanel>(descriptor),
            Err(PluginPanelRegistrationError::DuplicateStableId {
                stable_id: "org.example.camera/v1".to_owned(),
            })
        );
    }

    #[test]
    fn invalid_saved_panel_state_produces_one_user_facing_diagnostic() {
        let mut registry = PluginPanelRegistry::default();
        registry
            .register::<RejectingStatePanel>(PluginPanelDescriptor::new(
                "org.example.camera/v1",
                "Camera",
            ))
            .unwrap();
        let mut panels = PluginPanels::new(registry);
        panels.restore_state(PluginPanelsState {
            panels: HashMap::from([(
                "org.example.camera/v1".to_owned(),
                HashMap::from([("panel-1".to_owned(), serde_json::json!({ "version": 9 }))]),
            )]),
        });

        let context = egui::Context::default();
        context.begin_pass(egui::RawInput::default());
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("plugin-panel-state-test"),
            egui::UiBuilder::new(),
        );
        let first_warning = panels.show("org.example.camera/v1", "panel-1", &mut ui);
        let second_warning = panels.show("org.example.camera/v1", "panel-1", &mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();

        assert!(first_warning.is_some_and(|warning| {
            warning.to_string().contains("unsupported state version 9")
        }));
        assert!(second_warning.is_none());
    }
}
