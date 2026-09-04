use std::collections::HashSet;
use std::rc::Rc;

use super::*;
use crate::model::{Connection, SocketId};
use crate::support::{graph_color, graph_position};
use crate::widget::graph::interaction_state::snap_to_grid;
use crate::widget::graph::wire::node_has_any_connection;

fn modal_drag_bindings() -> std::sync::Arc<input_bindings::InputBindings> {
    std::sync::Arc::new(
            input_bindings::InputBindings::from_json(
                r#"{"bindings":[
                  {"context":"node_graph.drag_node","action":"confirm_move","label":"Confirm","input":"pointer","button":"primary","gesture":"release","any_modifiers":true},
                  {"context":"node_graph.drag_node","action":"cancel_move","label":"Cancel","input":"pointer","button":"secondary","gesture":"press","any_modifiers":true},
                  {"context":"node_graph.drag_wire","action":"confirm_link","label":"Confirm Link","input":"pointer","button":"primary","gesture":"release","any_modifiers":true},
                  {"context":"node_graph.drag_wire","action":"cancel_link","label":"Cancel","input":"pointer","button":"secondary","gesture":"press","any_modifiers":true}
                ]}"#,
            )
            .expect("modal drag bindings are valid"),
        )
}

fn socket(node: u32, index: usize, direction: SocketDirection) -> SocketId {
    SocketId {
        node: NodeId(node),
        index,
        direction,
    }
}

#[test]
fn outside_click_that_closes_a_menu_does_not_clear_node_selection() {
    use crate::runtime::NodeTypeRegistry;
    use crate::widget::graph::action::GraphAction;
    use crate::widget::menu::MenuEntry;

    fn show_frame(context: &egui::Context, widget: &mut NodeGraphWidget, events: Vec<egui::Event>) {
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("menu-dismiss-selection-test"),
            egui::UiBuilder::new().max_rect(screen_rect),
        );
        widget.show(&mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();
    }

    let context = egui::Context::default();
    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let node = widget
        .add_node_at("Reroute", Pos2::new(300.0, 250.0))
        .unwrap();
    widget.graph.nodes.get_mut(&node).unwrap().selected = true;
    show_frame(&context, &mut widget, Vec::new());

    widget.menu.open_popup(
        Pos2::new(400.0, 300.0),
        vec![MenuEntry::action("Unused", GraphAction::DeselectAll)],
    );
    let outside = Pos2::new(60.0, 60.0);
    show_frame(
        &context,
        &mut widget,
        vec![
            egui::Event::PointerMoved(outside),
            egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    show_frame(
        &context,
        &mut widget,
        vec![egui::Event::PointerButton {
            pos: outside,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(!widget.menu.is_open());
    assert!(widget.graph.nodes[&node].selected);
}

#[test]
fn node_with_no_connections_has_none() {
    let connections = vec![Connection {
        from: socket(1, 0, SocketDirection::Output),
        to: socket(2, 0, SocketDirection::Input),
    }];
    assert!(!node_has_any_connection(&connections, NodeId(3)));
}

#[test]
fn node_as_connection_source_counts() {
    let connections = vec![Connection {
        from: socket(1, 0, SocketDirection::Output),
        to: socket(2, 0, SocketDirection::Input),
    }];
    assert!(node_has_any_connection(&connections, NodeId(1)));
}

#[test]
fn node_as_connection_target_counts() {
    let connections = vec![Connection {
        from: socket(1, 0, SocketDirection::Output),
        to: socket(2, 0, SocketDirection::Input),
    }];
    assert!(node_has_any_connection(&connections, NodeId(2)));
}

#[test]
fn snap_to_grid_rounds_to_the_nearest_grid_point() {
    assert_eq!(
        snap_to_grid(Pos2::new(24.0, 26.0), 10.0),
        Pos2::new(20.0, 30.0)
    );
    assert_eq!(
        snap_to_grid(Pos2::new(-3.0, 5.0), 10.0),
        Pos2::new(0.0, 10.0)
    );
    assert_eq!(
        snap_to_grid(Pos2::new(10.0, 10.0), 10.0),
        Pos2::new(10.0, 10.0)
    );
}

#[test]
fn pressing_the_active_drag_axis_again_restores_free_movement() {
    let position = Pos2::new(13.0, 17.0);
    let x_constraint = toggle_drag_axis(None, DragAxis::X, position).unwrap();
    assert_eq!(x_constraint.axis, DragAxis::X);
    assert_eq!(x_constraint.locked_coordinate, 17.0);
    assert_eq!(
        toggle_drag_axis(Some(x_constraint), DragAxis::X, position),
        None
    );
    assert_eq!(
        toggle_drag_axis(Some(x_constraint), DragAxis::Y, position),
        Some(DragConstraint {
            axis: DragAxis::Y,
            locked_coordinate: 13.0,
        })
    );
}

#[test]
fn activating_a_constraint_keeps_the_other_axis_at_its_current_position() {
    let current = Pos2::new(24.0, 36.0);
    let pointer_position = Pos2::new(43.0, 58.0);
    let x_constraint = toggle_drag_axis(None, DragAxis::X, current);
    let y_constraint = toggle_drag_axis(None, DragAxis::Y, current);

    assert_eq!(
        constrain_drag_position(pointer_position, x_constraint, false),
        Pos2::new(43.0, 36.0)
    );
    assert_eq!(
        constrain_drag_position(pointer_position, y_constraint, false),
        Pos2::new(24.0, 58.0)
    );
    assert_eq!(
        constrain_drag_position(pointer_position, x_constraint, true),
        Pos2::new(40.0, 36.0)
    );
}

#[test]
fn disabling_a_constraint_rebases_free_movement_without_a_jump() {
    let current = Pos2::new(24.0, 36.0);
    let pointer = Pos2::new(43.0, 58.0);
    let offset = rebase_drag_offset(pointer, current);

    assert_eq!(
        constrain_drag_position((pointer.to_vec2() - offset).to_pos2(), None, false),
        current
    );
}

#[test]
fn secondary_press_always_cancels_even_when_another_button_is_configured() {
    use crate::runtime::NodeTypeRegistry;

    let context = egui::Context::default();
    context.begin_pass(egui::RawInput {
        events: vec![egui::Event::PointerButton {
            pos: Pos2::new(20.0, 20.0),
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    });
    let ui = egui::Ui::new(
        context.clone(),
        egui::Id::new("cancel-node-drag-test"),
        Default::default(),
    );
    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(std::sync::Arc::new(
            input_bindings::InputBindings::from_json(
                r#"{"bindings":[
                  {"context":"node_graph.drag_node","action":"cancel_move","label":"Cancel","input":"pointer","button":"primary","gesture":"press","any_modifiers":true}
                ]}"#,
            )
            .unwrap(),
        ));
    widget.interaction_state = InteractionState::DraggingNode {
        node_id: NodeId(1),
        offset: Vec2::ZERO,
        constraint: None,
    };

    assert!(widget.cancel_active_drag(&ui));
    assert!(matches!(widget.interaction_state, InteractionState::Idle));
    let mut output = context.end_pass();
    output.textures_delta.clear();
}

#[test]
fn cancelling_a_new_wire_drag_does_not_pop_an_unrelated_undo_step() {
    use crate::runtime::NodeTypeRegistry;

    let context = egui::Context::default();
    context.begin_pass(egui::RawInput {
        events: vec![egui::Event::PointerButton {
            pos: Pos2::new(20.0, 20.0),
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    });
    let ui = egui::Ui::new(
        context.clone(),
        egui::Id::new("cancel-new-wire-test"),
        Default::default(),
    );
    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.push_undo_snapshot();
    widget.interaction_state = InteractionState::DraggingWire {
        from: socket(1, 0, SocketDirection::Output),
        from_canvas: Pos2::ZERO,
        current_canvas: Pos2::new(10.0, 10.0),
        restore_on_cancel: false,
        connectable: Rc::new(HashSet::new()),
    };

    assert!(widget.cancel_active_drag(&ui));
    assert_eq!(widget.undo_stack.len(), 1);
    assert!(matches!(widget.interaction_state, InteractionState::Idle));
    let mut output = context.end_pass();
    output.textures_delta.clear();
}

#[test]
fn duplicate_primary_filter_restores_a_dragged_node() {
    use crate::runtime::NodeTypeRegistry;

    fn show_frame(
        context: &egui::Context,
        widget: &mut NodeGraphWidget,
        events: Vec<egui::Event>,
    ) -> Pos2 {
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("node-drag-sequence-test"),
            egui::UiBuilder::new().max_rect(screen_rect),
        );
        let origin = ui.available_rect_before_wrap().min;
        widget.show(&mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();
        origin
    }

    let context = egui::Context::default();
    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(modal_drag_bindings());
    let original = Pos2::new(100.0, 100.0);
    let node_id = widget
        .add_node_at("Reroute", original)
        .expect("built-in reroute node");
    let origin = show_frame(&context, &mut widget, Vec::new());
    let press = widget.build_layout(origin).node_screen_rects[&node_id].center();
    show_frame(
        &context,
        &mut widget,
        vec![
            egui::Event::PointerMoved(press),
            egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let dragged = press + egui::vec2(40.0, 30.0);
    show_frame(
        &context,
        &mut widget,
        vec![egui::Event::PointerMoved(dragged)],
    );
    assert!(matches!(
        widget.interaction_state,
        InteractionState::DraggingNode { .. }
    ));
    let dragged_further = dragged + egui::vec2(10.0, 5.0);
    show_frame(
        &context,
        &mut widget,
        vec![egui::Event::PointerMoved(dragged_further)],
    );
    assert_ne!(widget.graph.nodes[&node_id].pos, graph_position(original));

    let mut raw_input = egui::RawInput {
        events: vec![egui::Event::PointerButton {
            pos: dragged_further,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    assert!(widget.filter_modal_raw_input(&mut raw_input));
    assert!(raw_input.events.is_empty());

    assert!(matches!(widget.interaction_state, InteractionState::Idle));
    assert_eq!(widget.graph.nodes[&node_id].pos, graph_position(original));
}

#[test]
fn duplicate_primary_filter_cancels_a_new_wire() {
    use crate::runtime::NodeTypeRegistry;

    fn show_frame(
        context: &egui::Context,
        widget: &mut NodeGraphWidget,
        events: Vec<egui::Event>,
    ) -> Pos2 {
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("wire-drag-sequence-test"),
            egui::UiBuilder::new().max_rect(screen_rect),
        );
        let origin = ui.available_rect_before_wrap().min;
        widget.show(&mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();
        origin
    }

    let context = egui::Context::default();
    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(modal_drag_bindings());
    let source = widget
        .add_node_at("Reroute", Pos2::new(100.0, 100.0))
        .expect("built-in reroute node");
    let origin = show_frame(&context, &mut widget, Vec::new());
    let output = socket(source.0, 0, SocketDirection::Output);
    let press = widget.build_layout(origin).socket_screen_pos[&output];
    show_frame(
        &context,
        &mut widget,
        vec![
            egui::Event::PointerMoved(press),
            egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let dragged = press + egui::vec2(80.0, 40.0);
    show_frame(
        &context,
        &mut widget,
        vec![egui::Event::PointerMoved(dragged)],
    );
    assert!(matches!(
        widget.interaction_state,
        InteractionState::DraggingWire { .. }
    ));

    let mut raw_input = egui::RawInput {
        events: vec![egui::Event::PointerButton {
            pos: dragged,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    assert!(widget.filter_modal_raw_input(&mut raw_input));
    assert!(raw_input.events.is_empty());

    assert!(matches!(widget.interaction_state, InteractionState::Idle));
    assert!(widget.graph.connections.is_empty());
}

fn link_move_bindings() -> std::sync::Arc<input_bindings::InputBindings> {
    std::sync::Arc::new(
        input_bindings::InputBindings::from_json(
            r#"{"bindings":[
              {"context":"node_graph","action":"select_move","label":"Select / Move","input":"pointer","button":"primary","gesture":"drag"},
              {"context":"node_graph.socket","action":"connect","label":"Connect","input":"pointer","button":"primary","gesture":"drag","status":false},
              {"context":"node_graph.socket","action":"move_link","label":"Move Link","input":"pointer","button":"primary","gesture":"drag","modifiers":{"control":true},"status_modifier_only":true}
            ]}"#,
        )
        .expect("link move bindings are valid"),
    )
}

/// Drags away from the point `press_at` picks out of the laid-out graph,
/// with `modifiers` held for the whole gesture, and returns that point.
fn output_socket_pos(layout: &GraphWidgetLayout, node: NodeId) -> Pos2 {
    layout.socket_screen_pos[&socket(node.0, 0, SocketDirection::Output)]
}

fn drag_from(
    widget: &mut NodeGraphWidget,
    press_at: impl Fn(&GraphWidgetLayout) -> Pos2,
    modifiers: egui::Modifiers,
) -> Pos2 {
    fn show_frame(
        context: &egui::Context,
        widget: &mut NodeGraphWidget,
        modifiers: egui::Modifiers,
        events: Vec<egui::Event>,
    ) -> Pos2 {
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        // `ModifiersChanged` is how egui learns the held modifiers; it is
        // repeated each pass because `InputState` is rebuilt from events.
        let mut events = events;
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("link-move-sequence-test"),
            egui::UiBuilder::new().max_rect(screen_rect),
        );
        let origin = ui.available_rect_before_wrap().min;
        widget.show(&mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();
        origin
    }

    let context = egui::Context::default();
    let origin = show_frame(&context, widget, modifiers, Vec::new());
    let press = press_at(&widget.build_layout(origin));
    show_frame(
        &context,
        widget,
        modifiers,
        vec![
            egui::Event::PointerMoved(press),
            egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
        ],
    );
    show_frame(
        &context,
        widget,
        modifiers,
        vec![egui::Event::PointerMoved(press + egui::vec2(60.0, 40.0))],
    );
    press
}

/// Two reroutes wired source → target, ready for a drag off the source's
/// output socket.
fn connected_pair(widget: &mut NodeGraphWidget) -> (NodeId, NodeId) {
    let source = widget
        .add_node_at("Reroute", Pos2::new(100.0, 100.0))
        .expect("built-in reroute node");
    let target = widget
        .add_node_at("Reroute", Pos2::new(400.0, 260.0))
        .expect("built-in reroute node");
    widget.graph.add_connection(
        socket(source.0, 0, SocketDirection::Output),
        socket(target.0, 0, SocketDirection::Input),
    );
    (source, target)
}

#[test]
fn dragging_a_connected_output_adds_a_second_link() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(link_move_bindings());
    let (source, _target) = connected_pair(&mut widget);

    drag_from(
        &mut widget,
        |layout| output_socket_pos(layout, source),
        egui::Modifiers::NONE,
    );

    // The existing link survives: this drag is about to create another one.
    assert_eq!(widget.graph.connections.len(), 1);
    assert!(matches!(
        widget.interaction_state,
        InteractionState::DraggingWire { from, restore_on_cancel: false, .. }
            if from == socket(source.0, 0, SocketDirection::Output)
    ));
}

#[test]
fn ctrl_dragging_a_connected_output_picks_the_existing_link_up() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(link_move_bindings());
    let (source, target) = connected_pair(&mut widget);

    drag_from(
        &mut widget,
        |layout| output_socket_pos(layout, source),
        egui::Modifiers::CTRL,
    );

    // The drag now hangs from the input the link kept, so the free end can
    // be dropped on another output.
    let anchor = socket(target.0, 0, SocketDirection::Input);
    assert!(matches!(
        widget.interaction_state,
        InteractionState::DraggingWire { from, restore_on_cancel: false, .. }
            if from == anchor
    ));
    // The link itself stays in the document until the drag lands: it is the
    // wire being dragged, hidden rather than removed.
    assert_eq!(widget.graph.connections.len(), 1);
    assert_eq!(widget.moved_connection(anchor), Some(anchor));
}

#[test]
fn ctrl_dragging_an_unconnected_output_still_starts_a_new_link() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(link_move_bindings());
    let source = widget
        .add_node_at("Reroute", Pos2::new(100.0, 100.0))
        .expect("built-in reroute node");

    drag_from(
        &mut widget,
        |layout| output_socket_pos(layout, source),
        egui::Modifiers::CTRL,
    );

    assert!(matches!(
        widget.interaction_state,
        InteractionState::DraggingWire { from, restore_on_cancel: false, .. }
            if from == socket(source.0, 0, SocketDirection::Output)
    ));
}

#[test]
fn hovering_a_socket_reports_the_socket_binding_context() {
    use crate::runtime::NodeTypeRegistry;

    fn hover(widget: &mut NodeGraphWidget, at: Pos2) -> Option<&'static str> {
        let context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        for _ in 0..2 {
            context.begin_pass(egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![egui::Event::PointerMoved(at)],
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("socket-hover-context-test"),
                egui::UiBuilder::new().max_rect(screen_rect),
            );
            widget.show(&mut ui);
            let mut output = context.end_pass();
            output.textures_delta.clear();
        }
        widget.hovered_input_context()
    }

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let (source, target) = connected_pair(&mut widget);
    let layout = widget.build_layout(Pos2::ZERO);
    let connected_output = layout.socket_screen_pos[&socket(source.0, 0, SocketDirection::Output)];
    let free_output = layout.socket_screen_pos[&socket(target.0, 0, SocketDirection::Output)];
    let node_body = layout.node_screen_rects[&source].center();

    // Every socket reports the socket context — a drag there connects
    // rather than moving the node — and its hint names the link move.
    assert_eq!(
        hover(&mut widget, connected_output),
        Some("node_graph.socket")
    );
    assert_eq!(
        widget.status_hint(),
        "Drag to link · Ctrl+Drag to move an existing link"
    );
    assert_eq!(hover(&mut widget, free_output), Some("node_graph.socket"));
    assert_eq!(hover(&mut widget, node_body), Some("node_graph"));
}

#[test]
fn ctrl_dragging_a_reroute_point_picks_its_link_up_instead_of_moving_it() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    widget.set_input_bindings(link_move_bindings());
    let (source, target) = connected_pair(&mut widget);
    let placed = widget.graph.nodes[&source].pos;

    // The centre of a reroute is the sliver of body between its two socket
    // hit areas — a plain drag there moves the point.
    drag_from(
        &mut widget,
        |layout| layout.node_screen_rects[&source].center(),
        egui::Modifiers::CTRL,
    );

    assert_eq!(widget.graph.nodes[&source].pos, placed);
    let anchor = socket(target.0, 0, SocketDirection::Input);
    assert!(matches!(
        widget.interaction_state,
        InteractionState::DraggingWire { from, restore_on_cancel: false, .. }
            if from == anchor
    ));
    assert_eq!(widget.moved_connection(anchor), Some(anchor));
}

#[test]
fn dragging_a_reroute_point_still_moves_it() {
    use crate::runtime::NodeTypeRegistry;

    // Anywhere across the middle half of the point, not just its exact
    // centre: that whole band is the point's drag handle.
    for grip in [-0.2_f32, 0.0, 0.2] {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        widget.set_input_bindings(link_move_bindings());
        let (source, _target) = connected_pair(&mut widget);

        drag_from(
            &mut widget,
            |layout| {
                let body = layout.node_screen_rects[&source];
                body.center() + egui::vec2(body.width() * grip, 0.0)
            },
            egui::Modifiers::NONE,
        );

        assert_eq!(widget.graph.connections.len(), 1, "grip {grip}");
        assert!(
            matches!(
                widget.interaction_state,
                InteractionState::DraggingNode { node_id, .. } if node_id == source
            ),
            "grip {grip} did not move the point"
        );
    }
}

#[test]
fn a_wire_drag_stops_reporting_the_socket_it_was_pulled_off_as_hovered() {
    use crate::runtime::NodeTypeRegistry;

    /// The socket the graph considers hovered with the pointer resting on
    /// `source`'s output, optionally while a wire drag is under way.
    fn hovered_socket(dragging: bool) -> Option<SocketId> {
        let context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let (source, target) = connected_pair(&mut widget);
        let pulled_off = socket(source.0, 0, SocketDirection::Output);
        let anchor = socket(target.0, 0, SocketDirection::Input);
        let pointer = widget.build_layout(Pos2::ZERO).socket_screen_pos[&pulled_off];
        if dragging {
            widget.graph.disconnect_input(anchor);
            widget.interaction_state = InteractionState::DraggingWire {
                from: anchor,
                from_canvas: Pos2::new(600.0, 400.0),
                current_canvas: Pos2::new(300.0, 500.0),
                restore_on_cancel: true,
                connectable: Rc::new(HashSet::new()),
            };
        }

        let mut hovered = None;
        // Two passes: egui resolves hovering against the previous pass.
        for _ in 0..2 {
            context.begin_pass(egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("drag-hover-highlight-test"),
                egui::UiBuilder::new().max_rect(screen_rect),
            );
            let origin = ui.available_rect_before_wrap().min;
            widget.show(&mut ui);
            // Re-allocating the graph's own hit targets is how the test
            // reaches them; the canvas response is a placeholder that must
            // not cover the sockets it is asking about.
            let canvas = ui.interact(
                Rect::NOTHING,
                ui.id().with("hover-probe"),
                egui::Sense::hover(),
            );
            let layout = widget.build_layout(origin);
            let responses = widget.allocate_responses(&mut ui, canvas, &layout, screen_rect);
            hovered = widget.hovered_socket(&responses);
            let mut output = context.end_pass();
            output.textures_delta.clear();
        }
        hovered
    }

    // Idle, the socket under the pointer highlights itself and everything
    // it is wired to; mid-drag that socket is no longer part of the
    // gesture, so nothing of the old link stays lit.
    assert!(hovered_socket(false).is_some());
    assert_eq!(hovered_socket(true), None);
}

#[test]
fn only_a_link_creating_drag_shows_the_add_cursor() {
    use crate::runtime::NodeTypeRegistry;

    fn cursor_over_canvas(picked_up: bool) -> egui::CursorIcon {
        let context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(400.0, 300.0))
            .expect("built-in reroute node");
        let empty_canvas = Pos2::new(200.0, 520.0);

        context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![egui::Event::PointerMoved(empty_canvas)],
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("wire-drag-cursor-test"),
            egui::UiBuilder::new().max_rect(screen_rect),
        );
        let origin = ui.available_rect_before_wrap().min;
        // A picked-up link hangs from the input it kept; a new one starts
        // at the output it was pulled from.
        let from = if picked_up {
            socket(node.0, 0, SocketDirection::Input)
        } else {
            socket(node.0, 0, SocketDirection::Output)
        };
        let from_screen = widget.build_layout(origin).socket_screen_pos[&from];
        widget.interaction_state = InteractionState::DraggingWire {
            from,
            from_canvas: widget.view.screen_to_canvas(origin, from_screen),
            current_canvas: widget.view.screen_to_canvas(origin, empty_canvas),
            restore_on_cancel: picked_up,
            connectable: Rc::new(HashSet::new()),
        };
        widget.show(&mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();
        output.platform_output.cursor_icon
    }

    // The "+" promises a link that is about to exist; moving one keeps the
    // count unchanged, so it stays off.
    assert_eq!(cursor_over_canvas(false), egui::CursorIcon::Copy);
    assert_ne!(cursor_over_canvas(true), egui::CursorIcon::Copy);
}

#[test]
fn edge_auto_pan_does_nothing_well_inside_the_canvas() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
    widget.edge_auto_pan(canvas_rect.center(), canvas_rect);

    assert_eq!(widget.view.pan, Vec2::ZERO);
}

#[test]
fn edge_auto_pan_pans_positive_x_near_the_left_edge() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
    // Right at the left edge — well past the 24px margin.
    widget.edge_auto_pan(Pos2::new(0.0, canvas_rect.center().y), canvas_rect);

    assert!(widget.view.pan.x > 0.0);
    assert_eq!(widget.view.pan.y, 0.0);
}

#[test]
fn edge_auto_pan_pans_negative_x_near_the_right_edge() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
    widget.edge_auto_pan(Pos2::new(800.0, canvas_rect.center().y), canvas_rect);

    assert!(widget.view.pan.x < 0.0);
    assert_eq!(widget.view.pan.y, 0.0);
}

#[test]
fn edge_auto_pan_clamps_to_max_speed_past_the_edge() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
    // Far past the edge — overshoot would blow past MAX_SPEED unclamped.
    widget.edge_auto_pan(Pos2::new(-500.0, canvas_rect.center().y), canvas_rect);

    assert_eq!(widget.view.pan.x, 15.0);
}

#[test]
fn inserting_a_reroute_leaves_the_other_links_of_a_variadic_target_alone() {
    use crate::model::VariadicInfo;
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let first = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("built-in reroute node");
    let second = widget
        .add_node_at("Reroute", Pos2::new(0.0, 200.0))
        .expect("built-in reroute node");
    let sink = widget
        .add_node_at("Reroute", Pos2::new(400.0, 100.0))
        .expect("built-in reroute node");
    // A grown variadic group: two members and the trailing placeholder.
    {
        let node = widget.graph.nodes.get_mut(&sink).unwrap();
        let template = node.inputs[0].clone();
        node.inputs = ["D 1", "D 2", "D"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut socket = template.clone();
                socket.name = (*name).to_owned();
                socket.variadic = Some(VariadicInfo {
                    base: "D".to_owned(),
                    max: 8,
                    placeholder: index == 2,
                });
                socket
            })
            .collect();
    }
    let second_member = socket(sink.0, 1, SocketDirection::Input);
    widget.graph.add_connection(
        socket(first.0, 0, SocketDirection::Output),
        socket(sink.0, 0, SocketDirection::Input),
    );
    widget
        .graph
        .add_connection(socket(second.0, 0, SocketDirection::Output), second_member);

    let split = widget
        .graph
        .connections
        .iter()
        .position(|connection| connection.from.node == first)
        .expect("the first link is in the graph");
    widget.insert_reroute_on_wire(split, Pos2::new(200.0, 0.0));

    // Splitting one link must not disturb the group: had the original been
    // removed first, its member would have collapsed, renumbering the
    // sockets under the saved target and evicting the second link.
    assert_eq!(widget.graph.connections.len(), 3);
    assert_eq!(widget.graph.nodes[&sink].inputs.len(), 3);
    assert!(
        widget
            .graph
            .connections
            .iter()
            .any(|connection| connection.from.node == second && connection.to == second_member),
        "the second link lost its socket"
    );
}

#[test]
fn command_click_on_a_wire_inserts_a_reroute() {
    use crate::runtime::NodeTypeRegistry;

    fn reroute_bindings() -> std::sync::Arc<input_bindings::InputBindings> {
        std::sync::Arc::new(
            input_bindings::InputBindings::from_json(
                r#"{"bindings":[
                  {"context":"node_graph","action":"select_move","label":"Select / Move","input":"pointer","button":"primary","gesture":"drag"},
                  {"context":"node_graph.canvas","action":"insert_reroute","label":"Insert Reroute","input":"pointer","button":"primary","gesture":"double_click","status":false},
                  {"context":"node_graph.canvas","action":"insert_reroute","label":"Insert Reroute","input":"pointer","button":"primary","gesture":"click","modifiers":{"command":true},"status":false}
                ]}"#,
            )
            .expect("reroute bindings are valid"),
        )
    }

    /// Clicks `at` with `modifiers` held and reports the graph afterwards
    /// as (nodes, connections).
    fn click_on_wire(modifiers: egui::Modifiers) -> (usize, usize) {
        fn show_frame(
            context: &egui::Context,
            widget: &mut NodeGraphWidget,
            modifiers: egui::Modifiers,
            events: Vec<egui::Event>,
        ) {
            let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
            let mut events = events;
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            context.begin_pass(egui::RawInput {
                screen_rect: Some(screen_rect),
                events,
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("reroute-click-test"),
                egui::UiBuilder::new().max_rect(screen_rect),
            );
            widget.show(&mut ui);
            let mut output = context.end_pass();
            output.textures_delta.clear();
        }

        let context = egui::Context::default();
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        widget.set_input_bindings(reroute_bindings());
        let left = widget
            .add_node_at("Reroute", Pos2::new(0.0, 0.0))
            .expect("built-in reroute node");
        let right = widget
            .add_node_at("Reroute", Pos2::new(200.0, 0.0))
            .expect("built-in reroute node");
        widget.graph.add_connection(
            socket(left.0, 0, SocketDirection::Output),
            socket(right.0, 0, SocketDirection::Input),
        );
        // Both points' sockets sit on one horizontal line, so the wire runs
        // straight through here.
        let on_the_wire = Pos2::new(100.0, 12.0);

        show_frame(&context, &mut widget, modifiers, Vec::new());
        show_frame(
            &context,
            &mut widget,
            modifiers,
            vec![
                egui::Event::PointerMoved(on_the_wire),
                egui::Event::PointerButton {
                    pos: on_the_wire,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
        );
        show_frame(
            &context,
            &mut widget,
            modifiers,
            vec![egui::Event::PointerButton {
                pos: on_the_wire,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
        );

        (widget.graph.nodes.len(), widget.graph.connections.len())
    }

    // The wire is split by a third point carrying both halves.
    assert_eq!(click_on_wire(egui::Modifiers::COMMAND), (3, 2));
    // A plain click leaves it alone.
    assert_eq!(click_on_wire(egui::Modifiers::NONE), (2, 1));
}

#[test]
fn double_click_wire_inserts_a_reroute_and_rewires_both_halves() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let a = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("reroute should always be creatable");
    let b = widget
        .add_node_at("Reroute", Pos2::new(200.0, 0.0))
        .expect("reroute should always be creatable");
    let from = socket(a.0, 0, SocketDirection::Output);
    let to = socket(b.0, 0, SocketDirection::Input);
    widget.graph_mut().add_connection(from, to);

    let layout = widget.build_layout(Pos2::ZERO);
    // A and B's reroute sockets sit on a horizontal line at y=12
    // (REROUTE_SIZE/2); this point sits right on that wire.
    let click = Pos2::new(100.0, 12.0);
    let idx = widget
        .wire_near_point(click, &layout)
        .expect("click should land on the wire");
    widget.insert_reroute_on_wire(idx, click);

    assert_eq!(widget.graph.connections.len(), 2);
    assert_eq!(widget.graph.nodes.len(), 3);
    let new_id = *widget
        .graph
        .nodes
        .keys()
        .find(|&&id| id != a && id != b)
        .expect("a third node should have been inserted");
    assert_eq!(widget.graph.nodes[&new_id].pos, graph_position(click));
    assert!(
        widget
            .graph
            .connections
            .iter()
            .any(|c| c.from == from && c.to.node == new_id)
    );
    assert!(
        widget
            .graph
            .connections
            .iter()
            .any(|c| c.from.node == new_id && c.to == to)
    );
}

#[test]
fn connectable_nodes_includes_a_node_with_a_compatible_socket() {
    // Reroute sockets are `Any`/`Any`, so B's input is trivially
    // compatible with A's output — the smallest fixture that exercises
    // `connectable_nodes` without needing typed node defs (Phase 4.3).
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let a = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("reroute should always be creatable");
    let b = widget
        .add_node_at("Reroute", Pos2::new(200.0, 0.0))
        .expect("reroute should always be creatable");

    let connectable = widget.connectable_nodes(SocketId {
        node: a,
        index: 0,
        direction: SocketDirection::Output,
    });

    assert!(connectable.contains(&b));
}

#[test]
fn resolve_frame_membership_on_drop_never_ejects_a_current_member() {
    // Regression test: dragging can only ever *add* a node to a frame,
    // never remove it — no matter how far it's dragged. Removing is
    // exclusively the "Remove from Frame" action.
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let a = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("reroute should always be creatable");
    widget
        .graph
        .add_frame("F".to_owned(), graph_color(egui::Color32::WHITE), vec![a]);

    widget.graph.nodes.get_mut(&a).unwrap().pos = graph_position(Pos2::new(5000.0, 5000.0));
    let layout = widget.build_layout(Pos2::ZERO);
    widget.resolve_frame_membership_on_drop(&[a], &layout);

    assert_eq!(widget.graph.frames.len(), 1);
    assert!(widget.graph.frames[0].node_ids.contains(&a));
}

#[test]
fn resolve_frame_membership_on_drop_never_moves_a_member_to_a_different_frame() {
    // A node already in frame A must never switch to frame B via drag,
    // even when dropped squarely inside B's bounds — only nodes with no
    // current frame can join one this way.
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let a = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("reroute should always be creatable");
    let frame_a =
        widget
            .graph
            .add_frame("A".to_owned(), graph_color(egui::Color32::WHITE), vec![a]);
    let b = widget
        .add_node_at("Reroute", Pos2::new(5000.0, 5000.0))
        .expect("reroute should always be creatable");
    widget
        .graph
        .add_frame("B".to_owned(), graph_color(egui::Color32::WHITE), vec![b]);

    widget.graph.nodes.get_mut(&a).unwrap().pos = graph_position(Pos2::new(5000.0, 5000.0));
    let layout = widget.build_layout(Pos2::ZERO);
    widget.resolve_frame_membership_on_drop(&[a], &layout);

    let frame_a = widget
        .graph
        .frames
        .iter()
        .find(|f| f.id == frame_a)
        .expect("frame A should still exist");
    assert!(frame_a.node_ids.contains(&a));
}

#[test]
fn resolve_frame_membership_on_drop_joins_node_dropped_inside_a_frame() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let anchor = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("reroute should always be creatable");
    let frame_id = widget.graph.add_frame(
        "F".to_owned(),
        graph_color(egui::Color32::WHITE),
        vec![anchor],
    );
    let mover = widget
        .add_node_at("Reroute", Pos2::new(5000.0, 5000.0))
        .expect("reroute should always be creatable");

    widget.graph.nodes.get_mut(&mover).unwrap().pos = graph_position(Pos2::new(0.0, 0.0));
    let layout = widget.build_layout(Pos2::ZERO);
    widget.resolve_frame_membership_on_drop(&[mover], &layout);

    let frame = widget
        .graph
        .frames
        .iter()
        .find(|f| f.id == frame_id)
        .expect("frame should still exist");
    assert!(frame.node_ids.contains(&mover));
    assert!(frame.node_ids.contains(&anchor));
}

#[test]
fn resolve_frame_membership_on_drop_does_not_join_a_frame_from_outside_its_bounds() {
    use crate::runtime::NodeTypeRegistry;

    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let anchor = widget
        .add_node_at("Reroute", Pos2::new(0.0, 0.0))
        .expect("reroute should always be creatable");
    widget.graph.add_frame(
        "F".to_owned(),
        graph_color(egui::Color32::WHITE),
        vec![anchor],
    );
    let mover = widget
        .add_node_at("Reroute", Pos2::new(5000.0, 5000.0))
        .expect("reroute should always be creatable");

    // Left well outside F's bounds — should not join.
    let layout = widget.build_layout(Pos2::ZERO);
    widget.resolve_frame_membership_on_drop(&[mover], &layout);

    assert_eq!(widget.graph.frames.len(), 1);
    assert!(!widget.graph.frames[0].node_ids.contains(&mover));
}
