//! Blender-style properties panel (N-panel): a resizable strip docked to the
//! right border of the graph view. Widget-level tabs remain present for every
//! node; node definitions contribute opaque panel presentations to those tabs.
//! Widgets render in screen space
//! at full size, unaffected by graph zoom; edits mutate the same node state
//! as inline controls and run `on_update` through the same path.

use std::sync::Arc;

use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui,
    UiBuilder, Vec2,
};

use super::widget::NodeGraphWidget;
use crate::api::PanelContext;
use crate::model::{NodeId, NodeKind};

const PANEL_MIN_WIDTH: f32 = 220.0;
const PANEL_MAX_WIDTH: f32 = 520.0;
const TAB_BAR_WIDTH: f32 = 24.0;
const TAB_HEIGHT: f32 = 70.0;
const DEFAULT_ROW_HEIGHT: f32 = 24.0;
const PANEL_MARGIN_Y: f32 = 8.0;
const PANEL_TITLE_BLOCK_HEIGHT: f32 = 44.0;
const COLLAPSING_HEADER_HEIGHT: f32 = 26.0;
const PANEL_SECTION_GAP: f32 = 4.0;
const PANEL_MEASUREMENT_PADDING: f32 = 1.0;

pub(crate) struct PanelState {
    pub active_tab: Option<String>,
    pub width: f32,
    measured_node_height: Option<(NodeId, f32)>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            active_tab: Some("node".to_owned()),
            width: 300.0,
            measured_node_height: None,
        }
    }
}

impl NodeGraphWidget {
    pub(crate) fn toggle_panel(&mut self) {
        self.toggle_panel_tab("node");
    }

    fn toggle_panel_tab(&mut self, tab: &str) {
        self.panel.active_tab = if self.panel.active_tab.as_deref() == Some(tab) {
            None
        } else {
            Some(tab.to_owned())
        };
    }

    /// The selected node the panel shows: the active (most recently clicked)
    /// regular node while it remains selected, otherwise the newest selected
    /// regular node.
    fn panel_target(&self) -> Option<NodeId> {
        let shown = |id: &NodeId| {
            self.graph
                .nodes
                .get(id)
                .is_some_and(|node| node.kind == NodeKind::Regular && node.selected)
                && self.runtime.contains_key(id)
        };
        self.active_node.filter(shown).or_else(|| {
            self.graph
                .nodes
                .keys()
                .filter(|id| {
                    shown(id) && self.graph.nodes.get(id).is_some_and(|node| node.selected)
                })
                .max_by_key(|id| id.0)
                .copied()
        })
    }

    /// Screen rect occupied by the always-visible right-side tab strip.
    pub(crate) fn panel_tab_bar_rect(&self, canvas_rect: Rect) -> Rect {
        Rect::from_min_max(
            Pos2::new(canvas_rect.max.x - TAB_BAR_WIDTH, canvas_rect.min.y),
            canvas_rect.max,
        )
    }

    /// Screen rect the panel occupies this frame, `None` while hidden.
    pub(crate) fn panel_rect(&self, canvas_rect: Rect) -> Option<Rect> {
        self.panel.active_tab.as_ref()?;
        let width = self.panel.width.clamp(
            PANEL_MIN_WIDTH,
            (canvas_rect.width() - TAB_BAR_WIDTH - 160.0).max(PANEL_MIN_WIDTH),
        );
        let height = self.panel_height(canvas_rect);
        let tab_bar = self.panel_tab_bar_rect(canvas_rect);
        Some(Rect::from_min_max(
            Pos2::new(tab_bar.left() - width, canvas_rect.min.y),
            Pos2::new(tab_bar.left(), canvas_rect.min.y + height),
        ))
    }

    fn panel_height(&self, canvas_rect: Rect) -> f32 {
        let natural = if self.panel.active_tab.as_deref() == Some("node") {
            self.node_panel_height()
        } else if let Some(tab_id) = self.panel.active_tab.as_deref() {
            self.contributed_panel_height(tab_id)
        } else {
            0.0
        };
        natural.clamp(0.0, canvas_rect.height().max(0.0))
    }

    fn contributed_panel_height(&self, tab_id: &str) -> f32 {
        let Some(node_id) = self.panel_target() else {
            return 0.0;
        };
        let Some(instance) = self.runtime.get(&node_id) else {
            return 0.0;
        };
        let matching_panels = instance
            .panels()
            .into_iter()
            .enumerate()
            .filter(|(_, panel)| panel.tab_id == tab_id)
            .collect::<Vec<_>>();
        if matching_panels.is_empty() {
            return 0.0;
        }

        let panel_height = matching_panels
            .iter()
            .map(|(index, panel)| {
                let data = self.panel_data.get(&(node_id, panel.id.clone()));
                instance
                    .panel_preferred_height(*index, data.map(Arc::as_ref))
                    .unwrap_or(PANEL_TITLE_BLOCK_HEIGHT)
            })
            .sum::<f32>();
        PANEL_MARGIN_Y * 2.0 + panel_height + PANEL_SECTION_GAP * matching_panels.len() as f32
    }

    fn node_panel_height(&self) -> f32 {
        let Some(node_id) = self.panel_target() else {
            return PANEL_MARGIN_Y * 2.0 + PANEL_TITLE_BLOCK_HEIGHT;
        };
        if let Some((measured_node, height)) = self.panel.measured_node_height
            && measured_node == node_id
        {
            return height;
        }
        let mut height = PANEL_MARGIN_Y * 2.0
            + PANEL_TITLE_BLOCK_HEIGHT
            + COLLAPSING_HEADER_HEIGHT
            + 2.0 * DEFAULT_ROW_HEIGHT
            + PANEL_SECTION_GAP;

        if let Some(instance) = self.runtime.get(&node_id) {
            for section in instance.panel_sections() {
                height += COLLAPSING_HEADER_HEIGHT + PANEL_SECTION_GAP;
                height += section
                    .props
                    .iter()
                    .map(|prop| prop.height.unwrap_or(DEFAULT_ROW_HEIGHT))
                    .sum::<f32>();
            }
            for panel in instance
                .panels()
                .into_iter()
                .enumerate()
                .filter(|(_, panel)| panel.tab_id == "node")
            {
                let (index, panel) = panel;
                let data = self.panel_data.get(&(node_id, panel.id.clone()));
                height += instance
                    .panel_preferred_height(index, data.map(Arc::as_ref))
                    .unwrap_or(PANEL_TITLE_BLOCK_HEIGHT);
                height += PANEL_SECTION_GAP;
            }
        }

        height
    }

    /// Allocates the panel splitter. The body itself deliberately has no
    /// parent interaction response: its scroll area and child widgets own
    /// their pointer input, while `graph_pointer` keeps canvas gestures out.
    pub(crate) fn update_panel_interaction(&mut self, ui: &mut Ui, panel_rect: Rect) {
        let splitter_rect = Rect::from_min_max(
            Pos2::new(panel_rect.left() - 3.0, panel_rect.top()),
            Pos2::new(panel_rect.left() + 3.0, panel_rect.bottom()),
        );
        let splitter = ui.interact(
            splitter_rect,
            ui.id().with("props-panel-splitter"),
            Sense::click_and_drag(),
        );
        if splitter.hovered() || splitter.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if splitter.dragged()
            && let Some(pointer) = splitter.interact_pointer_pos()
        {
            self.panel.width =
                (panel_rect.right() - pointer.x).clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH);
        }
    }

    pub(crate) fn update_panel_tab_bar_interaction(&mut self, ui: &mut Ui, tab_bar_rect: Rect) {
        let tabs = self
            .panel_tabs
            .iter()
            .map(|tab| tab.id().to_owned())
            .collect::<Vec<_>>();
        for (index, tab_id) in tabs.iter().enumerate() {
            let response = ui.interact(
                self.panel_tab_rect(tab_bar_rect, index),
                ui.id().with(("props-panel-tab", tab_id)),
                Sense::click(),
            );
            if response.clicked() {
                self.toggle_panel_tab(tab_id);
            }
        }
    }

    pub(crate) fn show_panel_tab_bar(&self, ui: &mut Ui, tab_bar_rect: Rect) {
        let painter = ui.painter_at(tab_bar_rect);
        painter.rect_filled(tab_bar_rect, 0.0, Color32::from_rgb(31, 31, 31));
        painter.line_segment(
            [tab_bar_rect.left_top(), tab_bar_rect.left_bottom()],
            Stroke::new(1.0, Color32::from_rgb(62, 62, 62)),
        );

        for (index, tab) in self.panel_tabs.iter().enumerate() {
            let rect = self.panel_tab_rect(tab_bar_rect, index);
            let active = self.panel.active_tab.as_deref() == Some(tab.id());
            let fill = if active {
                Color32::from_rgb(58, 58, 58)
            } else {
                Color32::from_rgb(39, 39, 39)
            };
            let stroke = if active {
                Color32::from_rgb(92, 92, 92)
            } else {
                Color32::from_rgb(55, 55, 55)
            };
            painter.rect_filled(rect.shrink(1.0), 4.0, fill);
            painter.rect_stroke(
                rect.shrink(1.0),
                4.0,
                Stroke::new(1.0, stroke),
                egui::StrokeKind::Inside,
            );

            let text = tab.label();
            let color = if active {
                Color32::WHITE
            } else {
                Color32::from_rgb(185, 185, 185)
            };
            let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), color);
            let text_pos = rect.center() - galley.rect.center().to_vec2();
            let mut shape = egui::epaint::TextShape::new(text_pos, galley, color)
                .with_angle_and_anchor(-std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER);
            shape.override_text_color = Some(color);
            painter.add(shape);
        }
    }

    fn panel_tab_rect(&self, tab_bar_rect: Rect, index: usize) -> Rect {
        let top = tab_bar_rect.top() + 8.0 + index as f32 * (TAB_HEIGHT + 6.0);
        Rect::from_min_size(
            Pos2::new(tab_bar_rect.left(), top),
            Vec2::new(tab_bar_rect.width(), TAB_HEIGHT),
        )
    }

    pub(crate) fn show_active_panel(&mut self, ui: &mut Ui, panel_rect: Rect) {
        let Some(tab_id) = self.panel.active_tab.clone() else {
            return;
        };
        if tab_id == "node" {
            self.show_properties_panel(ui, panel_rect);
        } else {
            self.show_contributed_panels(ui, panel_rect, &tab_id);
        }
    }

    fn show_properties_panel(&mut self, ui: &mut Ui, panel_rect: Rect) {
        let Some(node_id) = self.panel_target() else {
            self.show_empty_node_panel(ui, panel_rect);
            return;
        };
        let contributed_panels =
            self.runtime
                .get(&node_id)
                .map(|instance| instance.panels())
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .filter(|(_, panel)| panel.tab_id == "node")
                .map(|(index, panel)| {
                    let data = self.panel_data.get(&(node_id, panel.id.clone())).cloned();
                    let height = self.runtime.get(&node_id).and_then(|instance| {
                        instance.panel_preferred_height(index, data.as_deref())
                    });
                    (index, panel.id, panel.metadata, data, height)
                })
                .collect::<Vec<_>>();

        let painter = ui.painter_at(panel_rect);
        painter.rect_filled(panel_rect, 0.0, Color32::from_rgb(38, 38, 38));
        painter.line_segment(
            [panel_rect.left_top(), panel_rect.left_bottom()],
            Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 70)),
        );

        let Some(node) = self.graph.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(instance) = self.runtime.get_mut(&node_id) else {
            return;
        };
        let category = self
            .registry
            .category_of(node.def_name())
            .unwrap_or("")
            .to_owned();
        let sections = instance.panel_sections();
        let editing_enabled = self.editing_enabled;

        let content = panel_rect.shrink2(Vec2::new(10.0, 8.0));
        let mut changed = false;
        let mut contributed_changed = false;
        let previous_title = node.title.clone();
        let mut pending_panel_action = None;
        let mut measured_height = None;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(content)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(panel_rect);
                let output = egui::ScrollArea::vertical()
                    .id_salt("props-panel-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.push_id(("props-panel", node_id.0), |ui| {
                            ui.label(RichText::new(&node.title).size(15.0).strong());
                            ui.label(
                                RichText::new(format!("{} · {}", node.def_name(), category))
                                    .size(11.0)
                                    .weak(),
                            );
                            ui.add_space(6.0);

                            // Built-in section: identity of the node itself.
                            egui::CollapsingHeader::new("Node")
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.add_enabled_ui(editing_enabled, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Name").size(11.0));
                                            ui.text_edit_singleline(&mut node.title);
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Color").size(11.0));
                                            ui.color_edit_button_srgba(&mut node.header_color);
                                        });
                                    });
                                });

                            for (section_index, section) in sections.iter().enumerate() {
                                egui::CollapsingHeader::new(section.title.as_str())
                                    .id_salt((
                                        "props-panel-section",
                                        section.title.as_str(),
                                        section_index,
                                    ))
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        for (prop_index, prop) in section.props.iter().enumerate() {
                                            ui.push_id(
                                                (
                                                    "props-panel-property",
                                                    section.title.as_str(),
                                                    section_index,
                                                    prop.id.as_str(),
                                                ),
                                                |ui| {
                                                    let height =
                                                        prop.height.unwrap_or(DEFAULT_ROW_HEIGHT);
                                                    let width = ui.available_width();
                                                    let (rect, _) = ui.allocate_exact_size(
                                                        Vec2::new(width, height),
                                                        Sense::hover(),
                                                    );
                                                    if ui
                                                        .add_enabled_ui(editing_enabled, |ui| {
                                                            instance.draw_panel_prop(
                                                                section_index,
                                                                prop_index,
                                                                ui,
                                                                rect,
                                                                panel_rect,
                                                            )
                                                        })
                                                        .inner
                                                    {
                                                        changed = true;
                                                    }
                                                },
                                            );
                                        }
                                    });
                            }

                            for (index, panel_id, metadata, data, height) in &contributed_panels {
                                ui.push_id(("node-tab-panel", panel_id), |ui| {
                                    let mut action = None;
                                    let height = height.unwrap_or_else(|| {
                                        metadata.height().unwrap_or_else(|| ui.available_height())
                                    });
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::new(ui.available_width(), height),
                                        Sense::hover(),
                                    );
                                    ui.scope_builder(
                                        UiBuilder::new()
                                            .max_rect(rect)
                                            .layout(Layout::top_down(Align::Min)),
                                        |ui| {
                                            let mut context = PanelContext::new(
                                                editing_enabled,
                                                data.as_deref(),
                                                &mut action,
                                            );
                                            let panel_changed =
                                                instance.draw_panel(*index, ui, &mut context);
                                            changed |= panel_changed;
                                            contributed_changed |= panel_changed;
                                        },
                                    );
                                    if let Some(payload) = action {
                                        pending_panel_action = Some((panel_id.clone(), payload));
                                    }
                                });
                            }
                        });
                    });
                measured_height = Some(
                    output.content_size.y.ceil() + PANEL_MARGIN_Y * 2.0 + PANEL_MEASUREMENT_PADDING,
                );
            },
        );

        if node.title != previous_title {
            changed |= instance.set_bound_title(&node.title);
        }

        if let Some(height) = measured_height {
            self.panel.measured_node_height = Some((node_id, height));
        }

        if changed {
            self.run_update(node_id);
        }
        self.contributed_panel_state_changed |= contributed_changed;
        if let Some((panel_id, payload)) = pending_panel_action {
            self.panel_action = Some(crate::api::PanelAction::new(node_id, panel_id, payload));
        }
    }

    fn show_empty_node_panel(&self, ui: &mut Ui, panel_rect: Rect) {
        let painter = ui.painter_at(panel_rect);
        painter.rect_filled(panel_rect, 0.0, Color32::from_rgb(38, 38, 38));
        painter.line_segment(
            [panel_rect.left_top(), panel_rect.left_bottom()],
            Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 70)),
        );
        let content = panel_rect.shrink2(Vec2::new(10.0, 8.0));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(content)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(panel_rect);
                ui.label(RichText::new("Node").size(15.0).strong());
                ui.label(RichText::new("No active node").size(11.0).weak());
            },
        );
    }

    fn show_contributed_panels(&mut self, ui: &mut Ui, panel_rect: Rect, tab_id: &str) {
        let painter = ui.painter_at(panel_rect);
        painter.rect_filled(panel_rect, 0.0, Color32::from_rgb(38, 38, 38));
        painter.line_segment(
            [panel_rect.left_top(), panel_rect.left_bottom()],
            Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 70)),
        );
        let Some(node_id) = self.panel_target() else {
            return;
        };
        let editing_enabled = self.editing_enabled;
        let panel_metas = self
            .runtime
            .get(&node_id)
            .map(|instance| instance.panels())
            .unwrap_or_default();
        let matching_panels = panel_metas
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.tab_id == tab_id)
            .map(|(index, panel)| {
                let data = self.panel_data.get(&(node_id, panel.id.clone())).cloned();
                let height = self
                    .runtime
                    .get(&node_id)
                    .and_then(|instance| instance.panel_preferred_height(index, data.as_deref()))
                    .unwrap_or(PANEL_TITLE_BLOCK_HEIGHT);
                (index, panel.id.clone(), panel.metadata, data, height)
            })
            .collect::<Vec<_>>();

        let content = panel_rect.shrink2(Vec2::new(10.0, 8.0));
        let mut changed = false;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(content)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(panel_rect);
                ui.push_id(("node-panels", node_id.0, tab_id), |ui| {
                    for (index, panel_id, metadata, data, height) in &matching_panels {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), *height),
                            Sense::hover(),
                        );
                        ui.scope_builder(
                            UiBuilder::new()
                                .max_rect(rect)
                                .layout(Layout::top_down(Align::Min)),
                            |ui| {
                                let mut action = None;
                                let mut context = PanelContext::new(
                                    editing_enabled,
                                    data.as_deref(),
                                    &mut action,
                                );
                                let mut draw = |ui: &mut Ui| {
                                    self.runtime.get_mut(&node_id).is_some_and(|instance| {
                                        instance.draw_panel(*index, ui, &mut context)
                                    })
                                };
                                let panel_changed = if metadata.has_scrollbar() {
                                    egui::ScrollArea::vertical()
                                        .id_salt(("node-panel-scroll", panel_id))
                                        .auto_shrink([false, false])
                                        .show(ui, draw)
                                        .inner
                                } else {
                                    draw(ui)
                                };
                                changed |= panel_changed;
                                if let Some(payload) = action {
                                    self.panel_action = Some(crate::api::PanelAction::new(
                                        node_id,
                                        panel_id.clone(),
                                        payload,
                                    ));
                                }
                            },
                        );
                        ui.add_space(PANEL_SECTION_GAP);
                    }
                });
            },
        );

        if changed {
            self.run_update(node_id);
            self.contributed_panel_state_changed = true;
        }
    }
}

#[cfg(test)]
mod panel_tests {
    use std::any::Any;

    use egui::{Pos2, Rect, Ui, Vec2};
    use serde::{Deserialize, Serialize};

    use super::NodeGraphWidget;
    use crate::api::{NodeDef, NodePanelDef, NodePanelPresentation, PanelContext};
    use crate::runtime::NodeTypeRegistry;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestState;

    struct DynamicHeightPanel;

    impl NodePanelPresentation<TestState> for DynamicHeightPanel {
        fn preferred_height(
            &self,
            _state: &TestState,
            data: Option<&(dyn Any + Send + Sync)>,
        ) -> Option<f32> {
            data.and_then(|data| data.downcast_ref::<f32>()).copied()
        }

        fn draw(
            &self,
            _state: &mut TestState,
            _ui: &mut Ui,
            context: &mut PanelContext<'_>,
        ) -> bool {
            context.data::<bool>().copied().unwrap_or(false)
        }
    }

    struct TestNode;

    impl NodeDef for TestNode {
        type State = TestState;

        fn name() -> &'static str {
            "Panel height test"
        }

        fn category() -> &'static str {
            "Test"
        }

        fn inputs() -> Vec<crate::api::InputDef<TestState>> {
            Vec::new()
        }

        fn outputs() -> Vec<crate::api::OutputDef<TestState>> {
            Vec::new()
        }

        fn state() -> TestState {
            TestState
        }

        fn panels() -> Vec<NodePanelDef<TestState>> {
            vec![NodePanelDef::new("dynamic", "view", DynamicHeightPanel)]
        }
    }

    #[test]
    fn contributed_tab_height_follows_its_panels_content_height() {
        let mut registry = NodeTypeRegistry::new();
        registry.register::<TestNode>();
        let mut widget = NodeGraphWidget::new(registry);
        let node = widget
            .add_node_at(TestNode::name(), Pos2::ZERO)
            .expect("test node is registered");
        widget
            .graph
            .nodes
            .get_mut(&node)
            .expect("test node exists")
            .selected = true;
        widget.set_panel_data(node, "dynamic", 80.0_f32);
        widget.panel.active_tab = Some("view".into());

        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 800.0));
        let panel = widget.panel_rect(canvas).expect("view panel is open");

        assert_eq!(panel.height(), 100.0);
    }

    #[test]
    fn contributed_panel_state_changes_are_reported_once_to_the_host() {
        let mut registry = NodeTypeRegistry::new();
        registry.register::<TestNode>();
        let mut widget = NodeGraphWidget::new(registry);
        let node = widget
            .add_node_at(TestNode::name(), Pos2::ZERO)
            .expect("test node is registered");
        widget
            .graph
            .nodes
            .get_mut(&node)
            .expect("test node exists")
            .selected = true;
        widget.set_panel_data(node, "dynamic", true);

        let context = egui::Context::default();
        let _ = context.run_ui(Default::default(), |ui| {
            widget.show_contributed_panels(
                ui,
                Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 200.0)),
                "view",
            );
        });

        assert!(widget.take_contributed_panel_state_changed());
        assert!(!widget.take_contributed_panel_state_changed());
    }

    #[test]
    fn node_panel_uses_measured_content_height_and_clamps_to_the_canvas() {
        let mut registry = NodeTypeRegistry::new();
        registry.register::<TestNode>();
        let mut widget = NodeGraphWidget::new(registry);
        let node = widget
            .add_node_at(TestNode::name(), Pos2::ZERO)
            .expect("test node is registered");
        widget
            .graph
            .nodes
            .get_mut(&node)
            .expect("test node exists")
            .selected = true;
        widget.panel.measured_node_height = Some((node, 480.0));

        let spacious_canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 800.0));
        assert_eq!(
            widget
                .panel_rect(spacious_canvas)
                .expect("node panel is open")
                .height(),
            480.0
        );

        let short_canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 300.0));
        assert_eq!(
            widget
                .panel_rect(short_canvas)
                .expect("node panel is open")
                .height(),
            300.0
        );
    }

    #[test]
    fn node_and_view_panels_stop_targeting_a_deselected_node() {
        let mut registry = NodeTypeRegistry::new();
        registry.register::<TestNode>();
        let mut widget = NodeGraphWidget::new(registry);
        let node = widget
            .add_node_at(TestNode::name(), Pos2::ZERO)
            .expect("test node is registered");
        widget
            .graph
            .nodes
            .get_mut(&node)
            .expect("test node exists")
            .selected = true;

        assert_eq!(widget.panel_target(), Some(node));

        widget
            .graph
            .nodes
            .get_mut(&node)
            .expect("test node exists")
            .selected = false;

        assert_eq!(widget.panel_target(), None);
    }
}
