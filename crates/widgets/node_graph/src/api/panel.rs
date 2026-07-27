use std::any::Any;

use egui::{RichText, Sense, Ui, Vec2};

use super::node::PanelSection;
use crate::model::NodeId;

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

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn panel_id(&self) -> &str {
        &self.panel_id
    }

    pub fn is<T: Any + Send>(&self) -> bool {
        self.payload.is::<T>()
    }

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
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

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
    pub fn preferred_height(mut self, height: f32) -> Self {
        self.preferred_height = Some(height.max(0.0));
        self
    }

    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    pub fn height(&self) -> Option<f32> {
        self.preferred_height
    }

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
    action: &'a mut Option<Box<dyn Any + Send>>,
}

impl<'a> PanelContext<'a> {
    pub(crate) fn new(
        editing_enabled: bool,
        data: Option<&'a (dyn Any + Send + Sync)>,
        action: &'a mut Option<Box<dyn Any + Send>>,
    ) -> Self {
        Self {
            editing_enabled,
            data,
            action,
        }
    }

    pub fn editing_enabled(&self) -> bool {
        self.editing_enabled
    }

    pub fn data<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.data.and_then(|data| data.downcast_ref())
    }

    pub fn emit<T: Any + Send>(&mut self, action: T) {
        *self.action = Some(Box::new(action));
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
    pub fn new(title: impl Into<String>, sections: Vec<PanelSection<S>>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            sections,
        }
    }

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
                                    prop.binding.draw(state, ui, rect, 1.0, ui.clip_rect())
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

    #[test]
    fn panel_data_and_actions_remain_typed_and_opaque() {
        let data = String::from("panel-owned");
        let mut action = None;
        let mut context = PanelContext::new(true, Some(&data), &mut action);

        assert_eq!(
            context.data::<String>().map(String::as_str),
            Some("panel-owned")
        );
        assert!(context.data::<u32>().is_none());
        context.emit(42_u32);

        assert_eq!(
            action.unwrap().downcast::<u32>().ok().map(|value| *value),
            Some(42)
        );
    }
}
