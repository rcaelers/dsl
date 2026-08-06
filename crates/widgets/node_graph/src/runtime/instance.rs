use std::any::Any;
use std::sync::Arc;

use egui::{Rect, Ui};
use serde_json::Value;

use super::registry::{reconcile_input_sockets, reconcile_output_sockets};
use crate::api::{
    FileDialogService, InputDef, NodeDef, NodePanelDef, OutputDef, PanelContext, PanelMetadata,
    PanelSection, PropDef,
};
use crate::model::{Node, NodeBadge, Socket};

/// Layout facts about one panel section, including the stable identity and
/// row height of each property.
pub(crate) struct PanelSectionMeta {
    pub(crate) title: String,
    pub(crate) props: Vec<PanelPropMeta>,
}

pub(crate) struct PanelPropMeta {
    pub(crate) id: String,
    pub(crate) height: Option<f32>,
}

pub(crate) struct NodePanelMeta {
    pub(crate) id: String,
    pub(crate) tab_id: String,
    pub(crate) metadata: PanelMetadata,
}

pub(crate) type NodeStateUpdate<S> =
    Arc<dyn Fn(&mut S, &mut [Socket], &mut [Socket]) + Send + Sync>;

pub(crate) trait NodeInstance {
    fn update(&mut self, inputs: &mut Vec<Socket>, outputs: &mut Vec<Socket>);
    fn badge(&self) -> Option<NodeBadge>;
    fn draw_input_control(
        &mut self,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool;
    fn draw_output_control(
        &mut self,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool;
    fn draw_property(
        &mut self,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool;
    fn panel_sections(&self) -> Vec<PanelSectionMeta>;
    fn panels(&self) -> Vec<NodePanelMeta>;
    fn panel_preferred_height(
        &self,
        index: usize,
        data: Option<&(dyn Any + Send + Sync)>,
    ) -> Option<f32>;
    fn draw_panel_prop(
        &mut self,
        section: usize,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool;
    fn draw_panel(&mut self, index: usize, ui: &mut Ui, context: &mut PanelContext<'_>) -> bool;
    fn bound_title(&mut self) -> Option<String>;
    fn set_bound_title(&mut self, title: &str) -> bool;
    fn save_state(&self) -> Value;
}

pub(crate) struct NodeRuntime {
    pub node: Node,
    pub instance: Box<dyn NodeInstance>,
}

pub(crate) struct TypedNode<T: NodeDef> {
    pub state: T::State,
    pub state_update: Option<NodeStateUpdate<T::State>>,
    pub inputs: Vec<InputDef<T::State>>,
    pub outputs: Vec<OutputDef<T::State>>,
    pub properties: Vec<PropDef<T::State>>,
    pub panel: Vec<PanelSection<T::State>>,
    pub panels: Vec<NodePanelDef<T::State>>,
}

impl<T: NodeDef> NodeInstance for TypedNode<T> {
    fn update(&mut self, inputs: &mut Vec<Socket>, outputs: &mut Vec<Socket>) {
        T::on_update(&mut self.state, inputs, outputs);
        if let Some(state_update) = &self.state_update {
            state_update(&mut self.state, inputs, outputs);
        }
        let schema = T::instance_schema(&self.state);
        reconcile_input_sockets(inputs, &schema.inputs);
        reconcile_output_sockets(outputs, &schema.outputs);
        self.inputs = schema.inputs;
        self.outputs = schema.outputs;
        self.properties = schema.props;
        self.panel = schema.panel;
        self.panels = schema.panels;
        T::on_update(&mut self.state, inputs, outputs);
        if let Some(state_update) = &self.state_update {
            state_update(&mut self.state, inputs, outputs);
        }
    }

    fn badge(&self) -> Option<NodeBadge> {
        T::badge(&self.state)
    }

    fn draw_input_control(
        &mut self,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool {
        self.inputs
            .get(index)
            .and_then(|input| input.control.as_ref())
            .is_some_and(|binding| {
                binding.draw(&mut self.state, ui, rect, zoom, clip_rect, file_dialog)
            })
    }

    fn draw_output_control(
        &mut self,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool {
        self.outputs
            .get(index)
            .and_then(|output| output.control.as_ref())
            .is_some_and(|binding| {
                binding.draw(&mut self.state, ui, rect, zoom, clip_rect, file_dialog)
            })
    }

    fn draw_property(
        &mut self,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool {
        self.properties.get(index).is_some_and(|property| {
            property
                .binding
                .draw(&mut self.state, ui, rect, zoom, clip_rect, file_dialog)
        })
    }

    fn panel_sections(&self) -> Vec<PanelSectionMeta> {
        panel_section_meta(&self.panel)
    }

    fn panels(&self) -> Vec<NodePanelMeta> {
        self.panels
            .iter()
            .map(|panel| NodePanelMeta {
                id: panel.id().to_owned(),
                tab_id: panel.tab_id().to_owned(),
                metadata: panel.panel_metadata(),
            })
            .collect()
    }

    fn panel_preferred_height(
        &self,
        index: usize,
        data: Option<&(dyn Any + Send + Sync)>,
    ) -> Option<f32> {
        self.panels
            .get(index)
            .and_then(|panel| panel.preferred_height(&self.state, data))
    }

    fn draw_panel_prop(
        &mut self,
        section: usize,
        index: usize,
        ui: &mut Ui,
        rect: Rect,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool {
        draw_panel_prop(
            &mut self.state,
            &self.panel,
            section,
            index,
            ui,
            rect,
            clip_rect,
            file_dialog,
        )
    }

    fn draw_panel(&mut self, index: usize, ui: &mut Ui, context: &mut PanelContext<'_>) -> bool {
        self.panels
            .get(index)
            .is_some_and(|panel| panel.draw(&mut self.state, ui, context))
    }

    fn bound_title(&mut self) -> Option<String> {
        T::title(&mut self.state).map(|title| title.value.clone())
    }

    fn set_bound_title(&mut self, title: &str) -> bool {
        let Some(bound) = T::title(&mut self.state) else {
            return false;
        };
        if bound.value == title {
            return false;
        }
        title.clone_into(&mut bound.value);
        true
    }

    fn save_state(&self) -> Value {
        serde_json::to_value(&self.state).expect("node state must serialize")
    }
}

fn panel_section_meta<S>(sections: &[PanelSection<S>]) -> Vec<PanelSectionMeta> {
    sections
        .iter()
        .map(|section| PanelSectionMeta {
            title: section.title.clone(),
            props: section
                .props
                .iter()
                .map(|prop| PanelPropMeta {
                    id: prop.id.clone(),
                    height: prop.panel_height,
                })
                .collect(),
        })
        .collect()
}

fn draw_panel_prop<S>(
    state: &mut S,
    sections: &[PanelSection<S>],
    section: usize,
    index: usize,
    ui: &mut Ui,
    rect: Rect,
    clip_rect: Rect,
    file_dialog: &mut dyn FileDialogService,
) -> bool {
    sections
        .get(section)
        .and_then(|section| section.props.get(index))
        .is_some_and(|prop| {
            // Panel widgets render in screen space at full size.
            prop.binding
                .draw(state, ui, rect, 1.0, clip_rect, file_dialog)
        })
}
