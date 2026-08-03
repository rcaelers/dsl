use std::any::Any;

use egui::{RichText, Sense, Ui, Vec2};

use super::control::FileDialogService;
use super::node::PanelSection;
use crate::model::NodeId;

/// Supplies host-owned, draw-scoped models to node-contributed panels.
///
/// The graph widget borrows returned values only for the current `show` call.
/// Providers remain responsible for model lifetime, replacement, and cleanup.
pub trait PanelDataProvider {
    /// Returns draw-scoped opaque data for one node panel.
    ///
    /// # Parameters
    /// - `node`: Node that owns the requested panel.
    /// - `panel_id`: Stable panel configuration identifier.
    fn panel_data(&self, node: NodeId, panel_id: &str) -> Option<&(dyn Any + Send + Sync)>;
}

/// An opaque action emitted by a node-contributed panel presentation.
pub struct PanelAction {
    node: NodeId,
    panel_id: String,
    payload: Box<dyn Any + Send>,
}

impl PanelAction {
    pub(crate) fn new(
        node: NodeId,
        panel_id: impl Into<String>,
        payload: Box<dyn Any + Send>,
    ) -> Self {
        Self {
            node,
            panel_id: panel_id.into(),
            payload,
        }
    }

    /// Returns the node that emitted this action.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Returns the stable identifier of the emitting panel.
    pub fn panel_id(&self) -> &str {
        &self.panel_id
    }

    /// Returns whether this action payload has type `T`.
    pub fn is<T: Any + Send>(&self) -> bool {
        self.payload.is::<T>()
    }

    /// Consumes and downcasts the opaque payload to `T`.
    pub fn downcast<T: Any + Send>(self) -> Result<T, Self> {
        match self.payload.downcast::<T>() {
            Ok(payload) => Ok(*payload),
            Err(payload) => Err(Self { payload, ..self }),
        }
    }
}

/// A tab in the node graph's docked panel area.
///
/// Tabs belong to the widget instance and remain visible independently of the
/// active node. IDs are stable configuration keys; labels are presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelTabDef {
    id: String,
    label: String,
}

impl PanelTabDef {
    /// Creates a stable tab definition for the docked panel area.
    ///
    /// # Parameters
    ///
    /// - `id`: Stable configuration key for the tab.
    /// - `label`: User-visible tab label.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }

    /// Returns the stable tab configuration key.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the user-visible tab label.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Layout policy for one node-contributed panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelMetadata {
    preferred_height: Option<f32>,
    scrollable: bool,
}

impl Default for PanelMetadata {
    fn default() -> Self {
        Self {
            preferred_height: None,
            scrollable: true,
        }
    }
}

impl PanelMetadata {
    /// Sets a non-negative preferred panel height.
    ///
    /// # Parameters
    /// - `height`: Desired height in screen points.
    pub fn preferred_height(mut self, height: f32) -> Self {
        self.preferred_height = Some(height.max(0.0));
        self
    }

    /// Sets whether the panel body may scroll.
    ///
    /// # Parameters
    ///
    /// - `scrollable`: Whether the widget should show scrolling when needed.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Returns the optional preferred panel height.
    pub fn height(&self) -> Option<f32> {
        self.preferred_height
    }

    /// Returns whether scrollbar.
    pub fn has_scrollbar(&self) -> bool {
        self.scrollable
    }
}

/// Application-neutral services available while a panel presentation draws.
/// Panel data and actions are opaque to the graph widget and interpreted only
/// by the concrete presentation and its host.
pub struct PanelContext<'a> {
    editing_enabled: bool,
    data: Option<&'a (dyn Any + Send + Sync)>,
    actions: &'a mut Vec<Box<dyn Any + Send>>,
    file_dialog: &'a mut dyn FileDialogService,
}

impl<'a> PanelContext<'a> {
    pub(crate) fn new(
        editing_enabled: bool,
        data: Option<&'a (dyn Any + Send + Sync)>,
        actions: &'a mut Vec<Box<dyn Any + Send>>,
        file_dialog: &'a mut dyn FileDialogService,
    ) -> Self {
        Self {
            editing_enabled,
            data,
            actions,
            file_dialog,
        }
    }

    /// Returns whether panel controls should permit state edits.
    pub fn editing_enabled(&self) -> bool {
        self.editing_enabled
    }

    /// Downcasts draw-scoped host data to the requested type.
    pub fn data<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.data.and_then(|data| data.downcast_ref())
    }

    /// Emits an opaque action for the host to interpret after drawing.
    pub fn emit<T: Any + Send>(&mut self, action: T) {
        self.actions.push(Box::new(action));
    }

    pub(crate) fn file_dialog(&mut self) -> &mut dyn FileDialogService {
        self.file_dialog
    }
}

/// Draws the complete contents of one node-contributed panel.
///
/// Titles, sections, controls, and empty-state presentation all belong to the
/// implementation. Returning `true` reports a node-state edit and causes the
/// normal `NodeDef::on_update` path to run.
pub trait NodePanelPresentation<S>: 'static {
    /// Returns the preferred screen-space height for this panel when it can
    /// derive one from its state or host-owned data. The widget otherwise uses
    /// the panel metadata height.
    fn preferred_height(&self, _state: &S, _data: Option<&(dyn Any + Send + Sync)>) -> Option<f32> {
        None
    }

    /// Draws this panel and returns whether it edited node state.
    ///
    /// # Parameters
    /// - `state`: Mutable concrete node state owned by the panel.
    /// - `ui`: Egui area in which to draw controls.
    /// - `context`: Draw-scoped data, action sink, and host services.
    fn draw(&self, state: &mut S, ui: &mut Ui, context: &mut PanelContext<'_>) -> bool;
}

/// Convenience presentation for node-owned property sections. Nodes remain
/// free to implement [`NodePanelPresentation`] directly for arbitrary UI.
pub struct PropertyPanelPresentation<S> {
    title: String,
    subtitle: Option<String>,
    sections: Vec<PanelSection<S>>,
}

impl<S> PropertyPanelPresentation<S> {
    /// Creates a property-section panel presentation.
    ///
    /// # Parameters
    ///
    /// - `title`: Primary panel title.
    /// - `sections`: Ordered node property sections to draw.
    pub fn new(title: impl Into<String>, sections: Vec<PanelSection<S>>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            sections,
        }
    }

    /// Adds a secondary explanatory line beneath the title.
    ///
    /// # Parameters
    ///
    /// - `subtitle`: User-visible supporting text.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

impl<S: 'static> NodePanelPresentation<S> for PropertyPanelPresentation<S> {
    fn preferred_height(&self, _state: &S, _data: Option<&(dyn Any + Send + Sync)>) -> Option<f32> {
        const TITLE_HEIGHT: f32 = 22.0;
        const SUBTITLE_HEIGHT: f32 = 16.0;
        const TITLE_GAP: f32 = 6.0;
        const SECTION_HEADER_HEIGHT: f32 = 26.0;
        const DEFAULT_ROW_HEIGHT: f32 = 24.0;

        let subtitle_height = self.subtitle.as_ref().map_or(0.0, |_| SUBTITLE_HEIGHT);
        let sections_height = self
            .sections
            .iter()
            .map(|section| {
                SECTION_HEADER_HEIGHT
                    + section
                        .props
                        .iter()
                        .map(|prop| prop.panel_height.unwrap_or(DEFAULT_ROW_HEIGHT))
                        .sum::<f32>()
            })
            .sum::<f32>();
        Some(TITLE_HEIGHT + subtitle_height + TITLE_GAP + sections_height)
    }

    fn draw(&self, state: &mut S, ui: &mut Ui, context: &mut PanelContext<'_>) -> bool {
        ui.label(RichText::new(&self.title).size(15.0).strong());
        if let Some(subtitle) = &self.subtitle {
            ui.label(RichText::new(subtitle).size(11.0).weak());
        }
        ui.add_space(6.0);

        let mut changed = false;
        for (section_index, section) in self.sections.iter().enumerate() {
            egui::CollapsingHeader::new(section.title.as_str())
                .id_salt(("node-panel-section", section.title.as_str(), section_index))
                .default_open(true)
                .show(ui, |ui| {
                    for prop in &section.props {
                        ui.push_id(("node-panel-property", prop.id.as_str()), |ui| {
                            let height = prop.panel_height.unwrap_or(24.0);
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), height),
                                Sense::hover(),
                            );
                            changed |= ui
                                .add_enabled_ui(context.editing_enabled(), |ui| {
                                    prop.binding.draw(
                                        state,
                                        ui,
                                        rect,
                                        1.0,
                                        ui.clip_rect(),
                                        context.file_dialog(),
                                    )
                                })
                                .inner;
                        });
                    }
                });
        }
        changed
    }
}

impl<S, F> NodePanelPresentation<S> for F
where
    F: Fn(&mut S, &mut Ui, &mut PanelContext<'_>) -> bool + 'static,
{
    fn draw(&self, state: &mut S, ui: &mut Ui, context: &mut PanelContext<'_>) -> bool {
        self(state, ui, context)
    }
}

/// A panel contributed by a concrete `NodeDef`.
pub struct NodePanelDef<S> {
    id: String,
    tab_id: String,
    metadata: PanelMetadata,
    presentation: Box<dyn NodePanelPresentation<S>>,
}

impl<S: 'static> NodePanelDef<S> {
    /// Creates one concrete node panel definition.
    ///
    /// # Parameters
    /// - `id`: Stable panel identifier within its node definition.
    /// - `tab_id`: Stable dock-tab identifier where this panel appears.
    /// - `presentation`: Concrete panel rendering implementation.
    pub fn new(
        id: impl Into<String>,
        tab_id: impl Into<String>,
        presentation: impl NodePanelPresentation<S>,
    ) -> Self {
        Self {
            id: id.into(),
            tab_id: tab_id.into(),
            metadata: PanelMetadata::default(),
            presentation: Box::new(presentation),
        }
    }

    /// Replaces layout metadata for this panel.
    ///
    /// # Parameters
    ///
    /// - `metadata`: Height and scrolling policy for this panel.
    pub fn metadata(mut self, metadata: PanelMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn tab_id(&self) -> &str {
        &self.tab_id
    }

    pub(crate) fn panel_metadata(&self) -> PanelMetadata {
        self.metadata
    }

    pub(crate) fn preferred_height(
        &self,
        state: &S,
        data: Option<&(dyn Any + Send + Sync)>,
    ) -> Option<f32> {
        self.presentation
            .preferred_height(state, data)
            .or(self.metadata.height())
    }

    pub(crate) fn draw(&self, state: &mut S, ui: &mut Ui, context: &mut PanelContext<'_>) -> bool {
        self.presentation.draw(state, ui, context)
    }
}

#[cfg(test)]
mod panel_context_tests {
    use super::*;
    use crate::api::UnavailableFileDialogService;

    #[test]
    fn panel_data_and_actions_remain_typed_and_opaque() {
        let data = String::from("panel-owned");
        let mut actions = Vec::new();
        let mut file_dialog = UnavailableFileDialogService;
        let mut context = PanelContext::new(true, Some(&data), &mut actions, &mut file_dialog);

        assert_eq!(
            context.data::<String>().map(String::as_str),
            Some("panel-owned")
        );
        assert!(context.data::<u32>().is_none());
        context.emit(42_u32);
        context.emit(43_u32);

        let values = actions
            .into_iter()
            .map(|action| *action.downcast::<u32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values, [42, 43]);
    }
}
