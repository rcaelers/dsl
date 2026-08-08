//! Inventory contract for one independently openable application panel.

use std::collections::HashSet;

use super::contract::{PluginPanel, PluginPanelDescriptor, PluginPanelIcon};
use super::error::{PluginPanelRegistrationError, validate_plugin_panel_definition};
use super::registry::PluginPanelRegistry;

/// Compile-time registration for one persistable panel kind.
pub struct UiPanelRegistration {
    stable_id: &'static str,
    title: &'static str,
    icon: PluginPanelIcon,
    minimum_width: f32,
    minimum_height: f32,
    singleton: bool,
    register: fn(
        &UiPanelRegistration,
        &mut PluginPanelRegistry,
    ) -> Result<(), PluginPanelRegistrationError>,
}

impl UiPanelRegistration {
    /// Creates an inventory registration for a default-constructible panel type.
    ///
    /// # Parameters
    /// - `stable_id`: Globally unique, persistable panel-kind identifier.
    /// - `title`: User-facing default title for instances of the panel.
    pub const fn panel<P: PluginPanel + Default + 'static>(
        stable_id: &'static str,
        title: &'static str,
    ) -> Self {
        Self {
            stable_id,
            title,
            icon: PluginPanelIcon::Panel,
            minimum_width: 180.0,
            minimum_height: 120.0,
            singleton: false,
            register: register_panel::<P>,
        }
    }

    /// Selects the icon shown for the registered panel.
    ///
    /// # Parameters
    /// - `icon`: Application-neutral icon supplied by the panel feature.
    pub const fn icon(mut self, icon: PluginPanelIcon) -> Self {
        self.icon = icon;
        self
    }

    /// Sets the minimum content size for panel-layout placement.
    ///
    /// # Parameters
    /// - `width`: Minimum width in logical points.
    /// - `height`: Minimum height in logical points.
    pub const fn minimum_size(mut self, width: f32, height: f32) -> Self {
        self.minimum_width = width;
        self.minimum_height = height;
        self
    }

    /// Restricts the registered panel kind to one visible instance.
    pub const fn singleton(mut self) -> Self {
        self.singleton = true;
        self
    }

    /// Returns the globally unique persistable panel-kind identifier.
    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    /// Returns the user-facing default panel title.
    pub const fn title(&self) -> &'static str {
        self.title
    }

    /// Validates the panel identity and user-facing title supplied by this registration.
    pub fn validate(&self) -> Result<(), PluginPanelRegistrationError> {
        validate_plugin_panel_definition(self.stable_id, self.title)
    }

    pub(crate) fn apply_to(
        &self,
        registry: &mut PluginPanelRegistry,
    ) -> Result<(), PluginPanelRegistrationError> {
        (self.register)(self, registry)
    }

    fn descriptor(&self) -> PluginPanelDescriptor {
        let mut descriptor = PluginPanelDescriptor::new(self.stable_id, self.title)
            .icon(self.icon)
            .minimum_size(self.minimum_width, self.minimum_height);
        if self.singleton {
            descriptor = descriptor.singleton();
        }
        descriptor
    }
}

fn register_panel<P: PluginPanel + Default + 'static>(
    registration: &UiPanelRegistration,
    registry: &mut PluginPanelRegistry,
) -> Result<(), PluginPanelRegistrationError> {
    registry.register::<P>(registration.descriptor())
}

inventory::collect!(UiPanelRegistration);

pub(crate) fn ui_panel_registrations()
-> Result<Vec<&'static UiPanelRegistration>, PluginPanelRegistrationError> {
    let mut registrations = inventory::iter::<UiPanelRegistration>
        .into_iter()
        .collect::<Vec<_>>();
    validate_ui_panel_registrations(&mut registrations)?;
    Ok(registrations)
}

fn validate_ui_panel_registrations(
    registrations: &mut Vec<&UiPanelRegistration>,
) -> Result<(), PluginPanelRegistrationError> {
    registrations.sort_by_key(|registration| registration.stable_id());

    let mut stable_ids = HashSet::new();
    for registration in registrations {
        registration.validate()?;
        if !stable_ids.insert(registration.stable_id()) {
            return Err(PluginPanelRegistrationError::DuplicateStableId {
                stable_id: registration.stable_id().to_owned(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod registration_tests {
    use super::super::contract::PluginPanelContext;
    use super::super::registry::PluginPanels;
    use super::*;

    #[derive(Default)]
    struct InventoryPanel;

    impl PluginPanel for InventoryPanel {
        fn show(&mut self, _ui: &mut egui::Ui, _context: PluginPanelContext<'_>) {}
    }

    inventory::submit! {
        UiPanelRegistration::panel::<InventoryPanel>(
            "org.logicconduit.test.inventory-panel/v1",
            "Inventory Panel",
        )
        .icon(PluginPanelIcon::Image)
    }

    #[test]
    fn inventory_panel_is_discovered_and_applied() {
        let registrations = ui_panel_registrations().unwrap();
        assert!(
            registrations
                .windows(2)
                .all(|pair| pair[0].stable_id() < pair[1].stable_id())
        );

        let panels = PluginPanels::new(PluginPanelRegistry::standard().unwrap());
        let definition = panels
            .definitions()
            .into_iter()
            .find(|definition| definition.stable_id == "org.logicconduit.test.inventory-panel/v1")
            .expect("inventory panel must be available to application composition");
        assert_eq!(definition.title, "Inventory Panel");
        assert_eq!(definition.icon, PluginPanelIcon::Image);
    }

    #[test]
    fn duplicate_ui_panel_registration_is_rejected() {
        let registration = ui_panel_registrations().unwrap()[0];
        let mut registrations = vec![registration, registration];

        assert_eq!(
            validate_ui_panel_registrations(&mut registrations),
            Err(PluginPanelRegistrationError::DuplicateStableId {
                stable_id: registration.stable_id().to_owned(),
            })
        );
    }

    #[test]
    fn plugin_can_classify_an_invalid_registration_before_submission() {
        let registration = UiPanelRegistration::panel::<InventoryPanel>(
            "org.logicconduit.test.invalid-panel/v1",
            " ",
        );

        assert_eq!(
            registration.validate(),
            Err(PluginPanelRegistrationError::EmptyTitle {
                stable_id: "org.logicconduit.test.invalid-panel/v1".to_owned(),
            })
        );
    }
}
