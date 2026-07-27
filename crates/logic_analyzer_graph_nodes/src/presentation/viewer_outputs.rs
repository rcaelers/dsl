use std::any::Any;

use egui::RichText;

use logic_analyzer_graph_api::node_support::{ViewerOutputPanelAction, ViewerOutputPanelModel};
use node_graph::{NodePanelDef, NodePanelPresentation, PanelContext};

struct ViewerOutputsPresentation;

impl<S: 'static> NodePanelPresentation<S> for ViewerOutputsPresentation {
    fn preferred_height(&self, _state: &S, data: Option<&(dyn Any + Send + Sync)>) -> Option<f32> {
        let model = data?.downcast_ref::<ViewerOutputPanelModel>()?;
        const TITLE_HEIGHT: f32 = 22.0;
        const TITLE_GAP: f32 = 6.0;
        const HEADER_HEIGHT: f32 = 26.0;
        const LANE_HEIGHT: f32 = 24.0;

        Some(TITLE_HEIGHT + TITLE_GAP + HEADER_HEIGHT + LANE_HEIGHT * model.outputs.len() as f32)
    }

    fn draw(&self, _state: &mut S, ui: &mut egui::Ui, context: &mut PanelContext<'_>) -> bool {
        ui.label(RichText::new("Viewer settings").size(15.0).strong());
        ui.add_space(6.0);
        let Some(model) = context.data::<ViewerOutputPanelModel>() else {
            return false;
        };
        let outputs = model.outputs.clone();
        egui::CollapsingHeader::new("Lanes")
            .default_open(true)
            .show(ui, |ui| {
                for output in &outputs {
                    let mut selected = output.selected;
                    if ui
                        .add_enabled(
                            context.editing_enabled(),
                            egui::Checkbox::new(&mut selected, &output.label),
                        )
                        .changed()
                    {
                        context.emit(ViewerOutputPanelAction::SetSelected {
                            id: output.id.clone(),
                            selected,
                        });
                    }
                }
            });
        false
    }
}

pub(crate) fn viewer_outputs_panel<S: 'static>() -> NodePanelDef<S> {
    NodePanelDef::new("viewer-outputs", "view", ViewerOutputsPresentation)
}
