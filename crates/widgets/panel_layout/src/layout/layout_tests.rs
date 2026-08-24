use super::*;
use crate::geometry::{
    BoundaryGeometry, adjacent_panels_at_boundary, panel_at_pointer, split_rects,
    title_interaction_rect,
};
use crate::tree::contains_content;

fn specs() -> [PanelSpec<'static>; 5] {
    [
        PanelSpec::new("viewer", "Viewer", 100.0).singleton(),
        PanelSpec::new("graph", "Graph", 100.0).singleton(),
        PanelSpec::new("watches", "Watches", 80.0),
        PanelSpec::new("triggers", "Triggers", 80.0),
        PanelSpec::new("decoder", "Decoder", 80.0).icon(PanelIcon::Table),
    ]
}

#[test]
fn initial_vertical_layout_preserves_requested_fraction() {
    let layout = PanelLayout::new([("viewer", 0.25), ("graph", 0.75)]);
    assert_eq!(layout.split_fraction("viewer", "graph"), Some(0.25));
}

#[test]
fn panel_icons_are_explicit_spec_metadata() {
    assert_eq!(
        PanelSpec::new("plain", "Plain", 100.0).icon,
        PanelIcon::Panel
    );
    assert_eq!(
        PanelSpec::new("signal", "Signal", 100.0)
            .icon(PanelIcon::Waveform)
            .icon,
        PanelIcon::Waveform
    );
}

#[test]
fn title_interaction_excludes_content_and_control_columns() {
    let title = Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(400.0, 42.0));
    let interaction = title_interaction_rect(title, 135.0, 370.0).unwrap();

    assert_eq!(interaction.left(), 135.0);
    assert_eq!(interaction.right(), 370.0);
    assert!(!interaction.contains(egui::pos2(100.0, 20.0)));
    assert!(!interaction.contains(egui::pos2(390.0, 20.0)));
    assert!(interaction.contains(egui::pos2(250.0, 20.0)));
    assert!(title_interaction_rect(title, 380.0, 370.0).is_none());
}

#[test]
fn rendered_title_interaction_avoids_host_widgets_and_panel_controls() {
    let context = egui::Context::default();
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0));
    context.begin_pass(egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
    });
    let mut ui = egui::Ui::new(
        context.clone(),
        egui::Id::new("title-interaction-test"),
        UiBuilder::new().max_rect(rect),
    );
    let mut layout = PanelLayout::new([("viewer", 1.0)]);

    let response = layout.show(&mut ui, rect, 0.0, &specs(), |slot, ui| {
        if matches!(slot, PanelSlot::TitleBar { .. }) {
            ui.label("Capture status");
            let _ = ui.small_button("Stop");
        }
    });
    let mut output = context.end_pass();
    output.textures_delta.clear();
    let panel = response.panel("viewer").unwrap();
    let interaction = panel.title_interaction_rect.unwrap();

    assert!(interaction.left() > panel.title_rect.left() + 44.0);
    assert!(interaction.right() < panel.title_rect.right());
    assert!(interaction.width() > 0.0);
}

#[test]
fn splitting_and_joining_mutates_only_the_split_tree() {
    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    layout.apply_action(
        LayoutAction::Split {
            panel_id: "viewer".to_owned(),
            axis: SplitAxis::Vertical,
            fraction: 0.3,
        },
        &specs(),
    );
    let root = layout.state.root.as_ref().unwrap();
    let LayoutNode::Split { first, .. } = root else {
        panic!("expected initial split");
    };
    let LayoutNode::Split { id, fraction, .. } = first.as_ref() else {
        panic!("expected nested split");
    };
    assert_eq!(*fraction, 0.3);
    let nested_id = *id;
    assert_eq!(all_panels(layout.state.root.as_ref()).len(), 3);

    layout.apply_action(
        LayoutAction::Join {
            split_id: nested_id,
            keep: SplitSide::First,
        },
        &specs(),
    );
    assert_eq!(all_panels(layout.state.root.as_ref()).len(), 2);
    assert!(find_panel_by_content(layout.state.root.as_ref(), "viewer").is_some());
}

#[test]
fn join_labels_describe_the_surviving_panels_expansion_direction() {
    assert_eq!(
        join_options(SplitAxis::Vertical),
        (
            ("Join Left", SplitSide::Second),
            ("Join Right", SplitSide::First),
        )
    );
    assert_eq!(
        join_options(SplitAxis::Horizontal),
        (
            ("Join Up", SplitSide::Second),
            ("Join Down", SplitSide::First),
        )
    );
}

#[test]
fn swapping_a_boundary_exchanges_only_the_adjacent_panels() {
    let mut layout = PanelLayout::new([("viewer", 0.4), ("graph", 0.6)]);
    assert!(layout.ensure_right_column_content("decoder", &["decoder"], 0.75));
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let (panels, boundaries) = layout.geometries(rect, &specs());
    let viewer = panels
        .iter()
        .find(|panel| panel.content_id == "viewer")
        .unwrap();
    let root_boundary = boundaries
        .iter()
        .find(|boundary| boundary.axis == SplitAxis::Vertical)
        .unwrap();
    let (viewer_panel_id, decoder_panel_id) =
        adjacent_panels_at_boundary(&panels, root_boundary, viewer.panel_rect.center()).unwrap();
    assert_eq!(
        find_panel(layout.state.root.as_ref(), &viewer_panel_id)
            .unwrap()
            .content,
        "viewer"
    );
    assert_eq!(
        find_panel(layout.state.root.as_ref(), &decoder_panel_id)
            .unwrap()
            .content,
        "decoder"
    );

    layout.apply_action(
        LayoutAction::SwapContent {
            first_panel_id: viewer_panel_id.clone(),
            second_panel_id: decoder_panel_id.clone(),
        },
        &specs(),
    );

    assert_eq!(
        find_panel(layout.state.root.as_ref(), &viewer_panel_id)
            .unwrap()
            .content,
        "decoder"
    );
    assert_eq!(
        find_panel(layout.state.root.as_ref(), &decoder_panel_id)
            .unwrap()
            .content,
        "viewer"
    );
    assert!(find_panel_by_content(layout.state.root.as_ref(), "graph").is_some());
}

#[test]
fn singleton_content_cannot_be_assigned_to_a_new_split() {
    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    layout.apply_action(
        LayoutAction::Split {
            panel_id: "viewer".to_owned(),
            axis: SplitAxis::Horizontal,
            fraction: 0.5,
        },
        &specs(),
    );
    let contents: Vec<_> = all_panels(layout.state.root.as_ref())
        .into_iter()
        .map(|panel| panel.content.as_str())
        .collect();
    assert_eq!(contents, ["viewer", "watches", "graph"]);
}

#[test]
fn split_target_follows_pointer_across_all_visible_panels() {
    let layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let (panels, _) = layout.geometries(rect, &specs());
    let viewer = panels
        .iter()
        .find(|panel| panel.content_id == "viewer")
        .unwrap();
    let graph = panels
        .iter()
        .find(|panel| panel.content_id == "graph")
        .unwrap();

    assert_eq!(
        panel_at_pointer(&panels, viewer.panel_rect.center())
            .map(|panel| panel.content_id.as_str()),
        Some("viewer")
    );
    assert_eq!(
        panel_at_pointer(&panels, graph.panel_rect.center()).map(|panel| panel.content_id.as_str()),
        Some("graph")
    );
}

#[test]
fn live_split_preview_renders_final_geometry_without_committing_state() {
    let layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let (preview, boundaries) = layout.split_preview_geometries(
        rect,
        &specs(),
        LayoutAction::Split {
            panel_id: "viewer".to_owned(),
            axis: SplitAxis::Vertical,
            fraction: 0.3,
        },
    );

    assert_eq!(preview.len(), 3);
    assert_eq!(boundaries.len(), 2);
    assert!(preview.iter().any(|panel| panel.content_id == "watches"));
    assert_eq!(all_panels(layout.state.root.as_ref()).len(), 2);
}

#[test]
fn adding_an_area_to_the_layout_wraps_the_complete_existing_tree() {
    for (side, fraction, expected_axis) in [
        (LayoutSide::Left, 0.25, SplitAxis::Vertical),
        (LayoutSide::Right, 0.75, SplitAxis::Vertical),
        (LayoutSide::Top, 0.25, SplitAxis::Horizontal),
        (LayoutSide::Bottom, 0.75, SplitAxis::Horizontal),
    ] {
        let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
        layout.apply_action(LayoutAction::SplitLayout { side, fraction }, &specs());

        let LayoutNode::Split {
            axis,
            fraction: actual_fraction,
            first,
            second,
            ..
        } = layout.state.root.as_ref().unwrap()
        else {
            panic!("expected a new root split");
        };
        assert_eq!(*axis, expected_axis);
        assert_eq!(*actual_fraction, fraction);
        let (new_area, previous_layout) = if side.new_area_is_first() {
            (first.as_ref(), second.as_ref())
        } else {
            (second.as_ref(), first.as_ref())
        };
        assert!(matches!(new_area, LayoutNode::Panel { .. }));
        assert!(contains_content(previous_layout, "viewer"));
        assert!(contains_content(previous_layout, "graph"));
    }
}

#[test]
fn full_height_side_area_preview_matches_the_committed_layout() {
    let layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let action = LayoutAction::SplitLayout {
        side: LayoutSide::Right,
        fraction: 0.75,
    };
    let (preview, preview_boundaries) =
        layout.split_preview_geometries(rect, &specs(), action.clone());
    let side_panel = preview
        .iter()
        .find(|panel| panel.content_id == "watches")
        .unwrap();

    assert_eq!(side_panel.panel_rect.top(), rect.top());
    assert_eq!(side_panel.panel_rect.bottom(), rect.bottom());
    assert_eq!(preview.len(), 3);
    assert_eq!(preview_boundaries.len(), 2);
    assert_eq!(all_panels(layout.state.root.as_ref()).len(), 2);

    let mut committed = layout;
    committed.apply_action(action, &specs());
    let (committed_panels, committed_boundaries) = committed.geometries(rect, &specs());
    assert_eq!(committed_panels.len(), preview.len());
    assert_eq!(committed_boundaries.len(), preview_boundaries.len());
}

#[test]
fn right_column_content_is_added_once_and_kept_in_declared_order() {
    for requested in [
        ["watches", "triggers", "decoder"],
        ["decoder", "triggers", "watches"],
    ] {
        let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
        for content in requested {
            assert!(layout.ensure_right_column_content(
                content,
                &["watches", "triggers", "decoder"],
                0.75,
            ));
        }
        assert!(!layout.ensure_right_column_content(
            "watches",
            &["watches", "triggers", "decoder"],
            0.75,
        ));

        let LayoutNode::Split {
            axis: SplitAxis::Vertical,
            first,
            second,
            ..
        } = layout.state.root.as_ref().unwrap()
        else {
            panic!("expected a right-side column");
        };
        assert!(contains_content(first, "viewer"));
        assert!(contains_content(first, "graph"));
        let column_contents: Vec<_> = all_panels(Some(second))
            .into_iter()
            .map(|panel| panel.content.as_str())
            .collect();
        assert_eq!(column_contents, ["watches", "triggers", "decoder"]);
        assert_eq!(all_panels(layout.state.root.as_ref()).len(), 5);
    }
}

#[test]
fn adjacent_content_restores_a_closed_primary_panel_inside_its_original_area() {
    let mut layout = PanelLayout::new([("logic_analyzer", 0.42), ("node_graph", 0.58)]);
    assert!(layout.ensure_right_column_content("watches", &["watches", "triggers"], 0.82,));
    layout.apply_panel_action("node_graph", PanelAction::Close);

    assert!(layout.ensure_adjacent_content(
        "node_graph",
        "logic_analyzer",
        SplitAxis::Horizontal,
        false,
        0.42,
    ));

    let LayoutNode::Split {
        axis: SplitAxis::Vertical,
        first,
        second,
        ..
    } = layout.state.root.as_ref().unwrap()
    else {
        panic!("expected the auxiliary column to remain outside the primary area");
    };
    let LayoutNode::Split {
        axis: SplitAxis::Horizontal,
        fraction,
        first: logic_analyzer,
        second: node_graph,
        ..
    } = first.as_ref()
    else {
        panic!("expected the restored primary split");
    };
    assert_eq!(*fraction, 0.42);
    assert!(contains_content(logic_analyzer, "logic_analyzer"));
    assert!(contains_content(node_graph, "node_graph"));
    assert!(contains_content(second, "watches"));
}

#[test]
fn right_column_can_contain_multiple_instances_of_one_content() {
    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);

    assert!(layout.ensure_right_column_content_count(
        "decoder",
        2,
        &["watches", "triggers", "decoder"],
        0.75,
    ));
    assert!(!layout.ensure_right_column_content_count(
        "decoder",
        2,
        &["watches", "triggers", "decoder"],
        0.75,
    ));

    let decoder_count = all_panels(layout.state.root.as_ref())
        .into_iter()
        .filter(|panel| panel.content == "decoder")
        .count();
    assert_eq!(decoder_count, 2);
}

#[test]
fn first_right_column_content_spans_the_complete_layout_height() {
    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    assert!(layout.ensure_right_column_content("watches", &["watches", "triggers"], 0.75,));
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let (panels, _) = layout.geometries(rect, &specs());
    let watches = panels
        .iter()
        .find(|panel| panel.content_id == "watches")
        .unwrap();

    assert_eq!(watches.panel_rect.top(), rect.top());
    assert_eq!(watches.panel_rect.bottom(), rect.bottom());
}

#[test]
fn maximizing_and_restoring_preserves_the_split_layout() {
    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    layout.apply_panel_action("graph", PanelAction::Maximize);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let (maximized, _) = layout.geometries(rect, &specs());
    assert_eq!(maximized.len(), 1);
    assert_eq!(maximized[0].content_id, "graph");

    layout.apply_panel_action("graph", PanelAction::RestoreMaximized);
    let (restored, _) = layout.geometries(rect, &specs());
    assert_eq!(restored.len(), 2);
}

#[test]
fn control_drag_aligns_a_boundary_with_a_parallel_neighbour() {
    let root = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let dragged = BoundaryGeometry {
        id: 2,
        axis: SplitAxis::Horizontal,
        rect: Rect::from_min_size(egui::pos2(600.0, 398.0), egui::vec2(200.0, 4.0)),
        parent_rect: Rect::from_min_size(egui::pos2(600.0, 0.0), egui::vec2(200.0, 600.0)),
    };
    let neighbour = BoundaryGeometry {
        id: 1,
        axis: SplitAxis::Horizontal,
        rect: Rect::from_min_size(egui::pos2(0.0, 298.0), egui::vec2(600.0, 4.0)),
        parent_rect: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 600.0)),
    };
    let boundaries = [dragged.clone(), neighbour];

    let fraction = dragged.snapped_fraction_at(egui::pos2(700.0, 307.0), &boundaries, root);
    let (_, splitter, _) = split_rects(
        dragged.parent_rect,
        SplitAxis::Horizontal,
        fraction,
        egui::Vec2::ZERO,
        egui::Vec2::ZERO,
        4.0,
    );

    assert_eq!(splitter.center().y, 300.0);
}

#[test]
fn control_drag_uses_the_layout_grid_away_from_neighbour_boundaries() {
    let root = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let dragged = BoundaryGeometry {
        id: 2,
        axis: SplitAxis::Horizontal,
        rect: Rect::from_min_size(egui::pos2(600.0, 398.0), egui::vec2(200.0, 4.0)),
        parent_rect: Rect::from_min_size(egui::pos2(600.0, 0.0), egui::vec2(200.0, 600.0)),
    };

    let fraction = dragged.snapped_fraction_at(
        egui::pos2(700.0, 333.0),
        std::slice::from_ref(&dragged),
        root,
    );
    let (_, splitter, _) = split_rects(
        dragged.parent_rect,
        SplitAxis::Horizontal,
        fraction,
        egui::Vec2::ZERO,
        egui::Vec2::ZERO,
        4.0,
    );

    assert_eq!(splitter.center().y, 336.0);
}

#[test]
fn shift_drag_lines_up_nearby_parallel_boundaries() {
    let root = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let dragged = BoundaryGeometry {
        id: 1,
        axis: SplitAxis::Horizontal,
        rect: Rect::from_min_size(egui::pos2(0.0, 298.0), egui::vec2(400.0, 4.0)),
        parent_rect: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 600.0)),
    };
    let neighbour = BoundaryGeometry {
        id: 2,
        axis: SplitAxis::Horizontal,
        rect: Rect::from_min_size(egui::pos2(400.0, 306.0), egui::vec2(400.0, 4.0)),
        parent_rect: Rect::from_min_size(egui::pos2(400.0, 0.0), egui::vec2(400.0, 600.0)),
    };
    let boundaries = [dragged.clone(), neighbour.clone()];

    let actions = dragged.resize_actions(egui::pos2(200.0, 350.0), &boundaries, root, false, true);
    let fractions: Vec<_> = actions
        .iter()
        .map(|action| match action {
            LayoutAction::SetFraction { split_id, fraction } => (*split_id, *fraction),
            _ => panic!("resize emitted a non-resize action"),
        })
        .collect();

    assert_eq!(fractions.len(), 2);
    assert_eq!(fractions[0].0, dragged.id);
    assert_eq!(fractions[1].0, neighbour.id);
    assert!((dragged.coordinate_for_fraction(fractions[0].1) - 350.0).abs() < 0.001);
    assert!((neighbour.coordinate_for_fraction(fractions[1].1) - 350.0).abs() < 0.001);
}

#[test]
fn option_break_transposes_an_aligned_grid_and_keeps_the_dragged_segment_id() {
    let panel = |id: &str| LayoutNode::Panel {
        panel: PanelState {
            id: id.to_owned(),
            content: id.to_owned(),
            title_bar_position: TitleBarPosition::Top,
        },
    };
    let mut root = LayoutNode::Split {
        id: 1,
        axis: SplitAxis::Vertical,
        fraction: 0.4,
        first: Box::new(LayoutNode::Split {
            id: 2,
            axis: SplitAxis::Horizontal,
            fraction: 0.5,
            first: Box::new(panel("top-left")),
            second: Box::new(panel("bottom-left")),
        }),
        second: Box::new(LayoutNode::Split {
            id: 3,
            axis: SplitAxis::Horizontal,
            fraction: 0.5,
            first: Box::new(panel("top-right")),
            second: Box::new(panel("bottom-right")),
        }),
    };

    assert!(break_split(Some(&mut root), 1, SplitSide::First, 0.5));

    let LayoutNode::Split {
        id,
        axis,
        first,
        second,
        ..
    } = root
    else {
        panic!("broken grid root is not a split");
    };
    assert_eq!((id, axis), (2, SplitAxis::Horizontal));
    assert!(matches!(
        first.as_ref(),
        LayoutNode::Split {
            id: 1,
            axis: SplitAxis::Vertical,
            ..
        }
    ));
    assert!(matches!(
        second.as_ref(),
        LayoutNode::Split {
            id: 3,
            axis: SplitAxis::Vertical,
            ..
        }
    ));
    let rebuilt = LayoutNode::Split {
        id,
        axis,
        fraction: 0.5,
        first,
        second,
    };
    let contents: Vec<_> = all_panels(Some(&rebuilt))
        .into_iter()
        .map(|panel| panel.content.as_str())
        .collect();
    assert_eq!(
        contents,
        ["top-left", "top-right", "bottom-left", "bottom-right"]
    );
}

#[test]
fn area_menu_uses_shared_shortcut_formatter() {
    for (key, expected) in [(egui::Key::Space, "^ Space"), (egui::Key::A, "^ A")] {
        let shortcut = KeyboardShortcut::new(egui::Modifiers::CTRL, key);
        assert_eq!(
            MenuShortcut::from_keyboard(shortcut).format(false),
            expected
        );
    }
}

#[test]
fn configured_shortcut_maximizes_hovered_area_and_then_restores() {
    fn press_shortcut(layout: &mut PanelLayout) {
        let context = egui::Context::default();
        let modifiers = egui::Modifiers::CTRL;
        let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        context.begin_pass(egui::RawInput {
            screen_rect: Some(rect),
            events: vec![
                egui::Event::PointerMoved(egui::pos2(100.0, 100.0)),
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: Some(egui::Key::Space),
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
            ],
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("panel-shortcut-test"),
            UiBuilder::new().max_rect(rect),
        );
        layout.set_maximize_shortcut(Some(KeyboardShortcut::new(modifiers, egui::Key::Space)));
        layout.show(&mut ui, rect, 0.0, &specs(), |_, _| {});
        let mut output = context.end_pass();
        output.textures_delta.clear();
    }

    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    press_shortcut(&mut layout);
    assert_eq!(layout.state.maximized.as_deref(), Some("viewer"));

    press_shortcut(&mut layout);
    assert_eq!(layout.state.maximized, None);
}

#[test]
fn title_bar_can_be_flipped_to_the_bottom() {
    let mut layout = PanelLayout::new([("viewer", 1.0)]);
    layout.apply_panel_action("viewer", PanelAction::FlipTitleBar);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let (panels, _) = layout.geometries(rect, &specs());
    let panel = &panels[0];

    assert_eq!(panel.title_bar_position, TitleBarPosition::Bottom);
    assert_eq!(panel.title_rect.bottom(), panel.panel_rect.bottom());
    assert_eq!(panel.body_rect.bottom(), panel.title_rect.top());
}

#[test]
fn closing_an_area_expands_its_sibling() {
    let mut layout = PanelLayout::new([("viewer", 0.5), ("graph", 0.5)]);
    layout.apply_panel_action("viewer", PanelAction::Close);

    let panels = all_panels(layout.state.root.as_ref());
    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].content, "graph");

    layout.apply_panel_action("graph", PanelAction::Close);
    assert_eq!(all_panels(layout.state.root.as_ref()).len(), 1);
}

#[test]
fn legacy_minimized_state_loads_as_a_regular_panel() {
    let json = r#"{
            "root": {
                "kind": "panel",
                "panel": {"id": "viewer", "content": "viewer", "minimized": true}
            },
            "maximized": null,
            "restore_minimized": [["viewer", true]],
            "next_id": 1
        }"#;
    let restored: PanelLayoutState = serde_json::from_str(json).unwrap();
    let panel = find_panel(restored.root.as_ref(), "viewer").unwrap();

    assert_eq!(panel.title_bar_position, TitleBarPosition::Top);
    let serialized = serde_json::to_string(&restored).unwrap();
    assert!(!serialized.contains("minimized"));
}

#[test]
fn state_round_trips_nested_layout() {
    let mut layout = PanelLayout::new([("viewer", 0.4), ("graph", 0.6)]);
    layout.apply_action(
        LayoutAction::Split {
            panel_id: "graph".to_owned(),
            axis: SplitAxis::Vertical,
            fraction: 0.7,
        },
        &specs(),
    );
    let json = serde_json::to_string(layout.state()).unwrap();
    let restored: PanelLayoutState = serde_json::from_str(&json).unwrap();
    assert_eq!(all_panels(restored.root.as_ref()).len(), 3);
}
