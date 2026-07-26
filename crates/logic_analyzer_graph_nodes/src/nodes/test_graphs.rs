//! Test graph fixtures for built-in graph-node integration tests.

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_graphs_tests {
    fn name(stable_id: &str) -> &'static str {
        crate::test_support::node_name(stable_id)
    }

    fn output_index(
        widget: &node_graph::NodeGraphWidget,
        node: node_graph::NodeId,
        name: &str,
    ) -> usize {
        widget.graph().nodes[&node]
            .outputs
            .iter()
            .position(|socket| socket.name == name)
            .unwrap_or_else(|| panic!("no output socket '{name}'"))
    }

    fn input_index(
        widget: &node_graph::NodeGraphWidget,
        node: node_graph::NodeId,
        name: &str,
    ) -> usize {
        widget.graph().nodes[&node]
            .inputs
            .iter()
            .position(|socket| socket.name == name && socket.visible)
            .unwrap_or_else(|| panic!("no input socket '{name}'"))
    }

    fn connect(
        widget: &mut node_graph::NodeGraphWidget,
        from: (node_graph::NodeId, &str),
        to: (node_graph::NodeId, &str),
    ) {
        let from_socket = node_graph::SocketId {
            node: from.0,
            index: output_index(widget, from.0, from.1),
            direction: node_graph::SocketDirection::Output,
        };
        let to_socket = node_graph::SocketId {
            node: to.0,
            index: input_index(widget, to.0, to.1),
            direction: node_graph::SocketDirection::Input,
        };
        widget.graph_mut().add_connection(from_socket, to_socket);
    }

    pub(crate) fn build_binary_decoder_demo(widget: &mut node_graph::NodeGraphWidget) {
        use egui::Pos2;
        let add = |widget: &mut node_graph::NodeGraphWidget, name: &str, x: f32, y: f32| {
            widget
                .add_node_at(name, Pos2::new(x, y))
                .unwrap_or_else(|| panic!("unknown node type '{name}'"))
        };

        let source_name = name("org.logicconduit.graph-node.sigrok-file-source/v1");
        let source = add(widget, source_name, 40.0, 300.0);
        let mut source_state = widget.graph().nodes[&source].state.clone();
        source_state["channel_names"] = serde_json::Value::Array(
            (0..11)
                .map(|channel| serde_json::Value::String(format!("Ch {channel}")))
                .collect(),
        );
        source_state["demo_data"] = true.into();
        widget.set_node_state(source, source_state);
        widget.graph_mut().nodes.get_mut(&source).unwrap().title = source_name.into();
        let spi = add(
            widget,
            name("org.logicconduit.graph-node.spi-decoder/v1"),
            360.0,
            80.0,
        );
        let start = add(
            widget,
            name("org.logicconduit.graph-node.word-matcher/v1"),
            680.0,
            40.0,
        );
        let stop = add(
            widget,
            name("org.logicconduit.graph-node.word-matcher/v1"),
            680.0,
            230.0,
        );
        let counter = add(
            widget,
            name("org.logicconduit.graph-node.counter/v1"),
            960.0,
            40.0,
        );
        let latch = add(
            widget,
            name("org.logicconduit.graph-node.sr-flip-flop/v1"),
            960.0,
            230.0,
        );
        let formatter = add(
            widget,
            name("org.logicconduit.graph-node.string-formatter/v1"),
            1240.0,
            40.0,
        );
        let gate = add(
            widget,
            name("org.logicconduit.graph-node.logic-gate/v1"),
            1198.4297,
            592.2656,
        );
        let decoder = add(
            widget,
            name("org.logicconduit.graph-node.binary-decoder/v1"),
            1520.0,
            300.0,
        );

        let matcher_state =
            |widget: &node_graph::NodeGraphWidget, node: node_graph::NodeId, pattern: &str| {
                let mut state = widget.graph().nodes[&node].state.clone();
                state["pattern"]["value"] = pattern.into();
                state["mask"]["value"] = "0xFF".into();
                state
            };
        widget.set_node_state(start, matcher_state(widget, start, "0x9A"));
        widget.set_node_state(stop, matcher_state(widget, stop, "0xDE"));

        let mut formatter_state = widget.graph().nodes[&formatter].state.clone();
        formatter_state["template"]["value"] = "Window {n:02}".into();
        widget.set_node_state(formatter, formatter_state);

        let mut decoder_state = widget.graph().nodes[&decoder].state.clone();
        decoder_state["input_strategy"]["value"] = "Packed stream".into();
        widget.set_node_state(decoder, decoder_state);

        for (id, title) in [
            (source, "Demo"),
            (start, "Match Start 0x9A"),
            (stop, "Match Stop 0xDE"),
            (gate, "Parallel Enable Gate"),
            (decoder, "Parallel Decoder"),
        ] {
            widget.graph_mut().nodes.get_mut(&id).unwrap().title = title.to_owned();
        }

        connect(widget, (source, "Ch 7"), (spi, "CLK"));
        connect(widget, (source, "Ch 6"), (spi, "MOSI"));
        connect(widget, (source, "Ch 5"), (spi, "MISO"));
        connect(widget, (source, "Ch 8"), (spi, "CS#"));
        connect(widget, (spi, "MOSI Words"), (start, "Words"));
        connect(widget, (spi, "MOSI Words"), (stop, "Words"));
        connect(widget, (start, "Match"), (latch, "Set"));
        connect(widget, (stop, "Match"), (latch, "Reset"));
        connect(widget, (start, "Match"), (counter, "Trigger"));
        connect(widget, (counter, "Count"), (formatter, "Value"));

        connect(widget, (source, "Ch 8"), (gate, "In"));
        connect(widget, (latch, "Q"), (gate, "In"));
        connect(widget, (gate, "Out"), (decoder, "Enable"));
        connect(widget, (source, "Ch 10"), (decoder, "Clock"));
        for bit in 0..8 {
            connect(widget, (source, &format!("Ch {bit}")), (decoder, "D"));
        }
        widget
            .graph_mut()
            .nodes
            .get_mut(&formatter)
            .unwrap()
            .selected = true;
    }

    /// Small two-channel graph used by live-capture cursor contract tests.
    pub(crate) fn build_live_binary_test(
        widget: &mut node_graph::NodeGraphWidget,
    ) -> node_graph::NodeId {
        use egui::Pos2;

        let source = widget
            .add_node_at(
                name("org.logicconduit.graph-node.test-live-capture-source/v1"),
                Pos2::new(40.0, 80.0),
            )
            .expect("demo source is registered");
        let decoder = widget
            .add_node_at(
                name("org.logicconduit.graph-node.binary-decoder/v1"),
                Pos2::new(360.0, 80.0),
            )
            .expect("binary decoder is registered");
        let mut decoder_state = widget.graph().nodes[&decoder].state.clone();
        decoder_state["input_strategy"]["value"] = "Packed stream".into();
        widget.set_node_state(decoder, decoder_state);
        connect(widget, (source, "Ch 0"), (decoder, "Clock"));
        connect(widget, (source, "Ch 1"), (decoder, "D"));
        source
    }

    /// Builds the CCD analysis pipeline captured by the embedded test fixture.
    /// Select a capture in its DSL File Source before running it (SPI cs=8
    /// clk=7 mosi=6; parallel strobe=10 (ACDK), data D0..D7 = ch 0..7).
    ///
    /// The enable gate is `AND(CS, Q)` with no NOT node: CS idles high and the
    /// parallel bus is decodable only while it is *inactive* (channels 6/7 are
    /// multiplexed with SPI), so the raw active-low line already is the
    /// "inverted SPI enable" of the requirement.
    pub(crate) fn populate_startup(widget: &mut node_graph::NodeGraphWidget) {
        use egui::Pos2;
        let add = |widget: &mut node_graph::NodeGraphWidget, name: &str, x: f32, y: f32| {
            widget
                .add_node_at(name, Pos2::new(x, y))
                .unwrap_or_else(|| panic!("unknown node type '{name}'"))
        };

        let source = add(
            widget,
            name("org.logicconduit.graph-node.dsl-file-source/v1"),
            40.0,
            260.0,
        );
        let mut source_state = widget.graph().nodes[&source].state.clone();
        source_state["channel_names"] = serde_json::Value::Array(
            (0..11)
                .map(|channel| serde_json::Value::String(format!("Ch {channel}")))
                .collect(),
        );
        widget.set_node_state(source, source_state);
        let spi = add(
            widget,
            name("org.logicconduit.graph-node.spi-decoder/v1"),
            330.0,
            120.0,
        );
        let start = add(
            widget,
            name("org.logicconduit.graph-node.word-matcher/v1"),
            620.0,
            40.0,
        );
        let stop = add(
            widget,
            name("org.logicconduit.graph-node.word-matcher/v1"),
            620.0,
            230.0,
        );
        let counter = add(
            widget,
            name("org.logicconduit.graph-node.counter/v1"),
            900.0,
            40.0,
        );
        let latch = add(
            widget,
            name("org.logicconduit.graph-node.sr-flip-flop/v1"),
            900.0,
            230.0,
        );
        let formatter = add(
            widget,
            name("org.logicconduit.graph-node.string-formatter/v1"),
            1160.0,
            40.0,
        );
        let gate = add(
            widget,
            name("org.logicconduit.graph-node.logic-gate/v1"),
            1160.0,
            400.0,
        );
        let decoder = add(
            widget,
            name("org.logicconduit.graph-node.binary-decoder/v1"),
            1440.0,
            260.0,
        );
        let writer = add(
            widget,
            name("org.logicconduit.graph-node.file-writer/v1"),
            1760.0,
            120.0,
        );

        // Configure states before wiring so `on_update`-driven socket visibility
        // (e.g. hidden MISO) is settled.
        let mut spi_state = widget.graph().nodes[&spi].state.clone();
        spi_state["word_size"]["value"] = 24.into();
        spi_state["has_miso"]["value"] = false.into();
        widget.set_node_state(spi, spi_state);
        let matcher_state =
            |widget: &node_graph::NodeGraphWidget, node: node_graph::NodeId, pattern: &str| {
                let mut state = widget.graph().nodes[&node].state.clone();
                state["pattern"]["value"] = pattern.into();
                state["mask"]["value"] = "0xFFFFFF".into();
                state
            };
        widget.set_node_state(start, matcher_state(widget, start, "0x600081"));
        widget.set_node_state(stop, matcher_state(widget, stop, "0x600000"));
        let mut decoder_state = widget.graph().nodes[&decoder].state.clone();
        decoder_state["sample_on"]["value"] = "Both (DDR)".into();
        decoder_state["word_size"]["value"] = 1.into();
        widget.set_node_state(decoder, decoder_state);

        // Descriptive titles (the def is still identified by `type_name`).
        for (id, title) in [
            (start, "Match Start"),
            (stop, "Match Stop"),
            (gate, "Enable Gate"),
        ] {
            if let Some(node) = widget.graph_mut().nodes.get_mut(&id) {
                node.title = title.to_owned();
            }
        }

        // SPI control path.
        connect(widget, (source, "Ch 7"), (spi, "CLK"));
        connect(widget, (source, "Ch 6"), (spi, "MOSI"));
        connect(widget, (source, "Ch 8"), (spi, "CS#"));
        connect(widget, (spi, "MOSI Words"), (start, "Words"));
        connect(widget, (spi, "MOSI Words"), (stop, "Words"));
        connect(widget, (start, "Match"), (latch, "Set"));
        connect(widget, (stop, "Match"), (latch, "Reset"));

        // Filename path.
        connect(widget, (start, "Match"), (counter, "Trigger"));
        connect(widget, (counter, "Count"), (formatter, "Value"));
        connect(widget, (formatter, "Text"), (writer, "Filename"));

        // Enable gate: stream window (Q) AND bus free (CS inactive-high).
        connect(widget, (source, "Ch 8"), (gate, "In"));
        connect(widget, (latch, "Q"), (gate, "In"));
        connect(widget, (gate, "Out"), (decoder, "Enable"));

        // Data path.
        connect(widget, (source, "Ch 10"), (decoder, "Clock"));
        for bit in 0..8 {
            connect(widget, (source, &format!("Ch {bit}")), (decoder, "D"));
        }
        connect(widget, (decoder, "Words"), (writer, "Data"));
    }

    /// Startup graph for the built-in UART demo. The signal is generated by a
    /// runtime source node; decoded words appear in the viewer only after the
    /// graph runs through the normal pipeline.
    pub(crate) fn populate_uart_demo(widget: &mut node_graph::NodeGraphWidget) {
        use egui::Pos2;
        let add = |widget: &mut node_graph::NodeGraphWidget, name: &str, x: f32, y: f32| {
            widget
                .add_node_at(name, Pos2::new(x, y))
                .unwrap_or_else(|| panic!("unknown node type '{name}'"))
        };

        let source = add(
            widget,
            name("org.logicconduit.graph-node.test-uart-source/v1"),
            80.0,
            220.0,
        );
        let uart = add(
            widget,
            name("org.logicconduit.graph-node.uart-decoder/v1"),
            420.0,
            180.0,
        );
        let viewer = add(
            widget,
            name("org.logicconduit.graph-node.viewer/v1"),
            760.0,
            230.0,
        );

        let mut source_state = widget.graph().nodes[&source].state.clone();
        source_state["message"]["value"] = "HELLO\n".into();
        source_state["baud_rate"]["value"] = 115_200.into();
        widget.set_node_state(source, source_state);
        let mut uart_state = widget.graph().nodes[&uart].state.clone();
        uart_state["baud_rate"]["value"] = 115_200.into();
        uart_state["data_bits"]["value"] = 8.into();
        uart_state["parity"]["value"] = "None".into();
        uart_state["check_parity"]["value"] = false.into();
        uart_state["stop_bits"]["value"] = "1".into();
        uart_state["bit_order"]["value"] = "LSB first".into();
        uart_state["invert"]["value"] = false.into();
        uart_state["error_output"]["value"] = false.into();
        widget.set_node_state(uart, uart_state);

        if let Some(node) = widget.graph_mut().nodes.get_mut(&source) {
            node.title = "Generated serial.rx".to_owned();
        }
        if let Some(node) = widget.graph_mut().nodes.get_mut(&uart) {
            node.title = "UART 115200 8N1".to_owned();
        }

        connect(widget, (source, "RX"), (uart, "RX/TX"));
        connect(widget, (source, "RX"), (viewer, "In"));
        connect(widget, (uart, "Data"), (viewer, "In"));
    }
}

#[cfg(test)]
mod tests {
    use logic_analyzer_graph_api::node::{RuntimeBuilder, graph_node_registrations};
    use logic_analyzer_graph_api::node_support::{CapturePresentation, ViewerOutputControl};
    use logic_analyzer_graph_compiler::{CompiledGraph, GraphCompiler};
    use node_graph::{GraphState, NodeGraphWidget, NodeId};

    use super::test_graphs_tests;
    use crate::test_support::build_registry as build_node_registry;

    fn selected_output_nodes(graph: &GraphState) -> Vec<NodeId> {
        let builders: std::collections::HashMap<String, Box<dyn RuntimeBuilder>> =
            graph_node_registrations()
                .into_iter()
                .filter_map(|registration| {
                    registration
                        .builder()
                        .map(|builder| (registration.name().to_owned(), builder))
                })
                .collect();
        graph
            .nodes
            .iter()
            .flat_map(|(&node_id, node)| {
                let builder = builders.get(node.def_name());
                node.outputs.iter().filter_map(move |output| {
                    let builder = builder?;
                    let ViewerOutputControl::Selectable {
                        default_selected, ..
                    } = builder.viewer_output_control(output, &node.state)?
                    else {
                        return None;
                    };
                    let selected = output
                        .extensions
                        .get("show_in_view")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(default_selected);
                    selected.then_some(node_id)
                })
            })
            .collect()
    }

    #[test]
    fn startup_graph_builds_with_compatible_wiring() {
        let mut widget = NodeGraphWidget::new(build_node_registry());
        test_graphs_tests::populate_startup(&mut widget);
        let graph = widget.graph();

        assert_eq!(graph.nodes.len(), 10);
        // src→spi 3, spi→matchers 2, matchers→latch 2, start→counter 1,
        // counter→formatter→writer 2, gate ins 2, gate→decoder 1,
        // clock 1, D0–D7 8, decoder→writer 1. Viewer lanes use the generic
        // watched-output flags and therefore add no saved connections.
        assert_eq!(graph.connections.len(), 23);

        for connection in &graph.connections {
            let from = &graph.nodes[&connection.from.node].outputs[connection.from.index];
            let to = &graph.nodes[&connection.to.node].inputs[connection.to.index];
            assert!(
                to.accepts(from.effective_type()),
                "incompatible wire {} ({}) -> {} ({})",
                from.name,
                from.effective_type(),
                to.name,
                to.type_name,
            );
        }
    }

    #[test]
    fn binary_decoder_demo_fixture_lowers() {
        let mut widget = NodeGraphWidget::new(build_node_registry());
        test_graphs_tests::build_binary_decoder_demo(&mut widget);
        assert!(widget.graph().nodes.values().all(|node| node.type_name
            != super::super::test_support::node_name("org.logicconduit.graph-node.viewer/v1")));
        let source_name = super::super::test_support::node_name(
            "org.logicconduit.graph-node.sigrok-file-source/v1",
        );
        let compiler = GraphCompiler::new();
        let selected_nodes = selected_output_nodes(widget.graph());
        let raw_channels = selected_nodes
            .iter()
            .filter(|node| widget.graph().nodes[node].def_name() == source_name)
            .count();
        let derived_lanes = selected_nodes
            .iter()
            .filter(|node| widget.graph().nodes[node].def_name() != source_name)
            .count();
        assert_eq!(raw_channels, 11);
        assert_eq!(derived_lanes, 0);
        let preview = compiler
            .discover_capture_presentation(widget.graph())
            .unwrap()
            .expect("demo source should provide a pre-run capture preview");
        let CapturePresentation::InMemory {
            signals: preview, ..
        } = preview.presentation
        else {
            panic!("demo source should provide an in-memory presentation");
        };
        assert_eq!(preview.len(), 10);
        assert_eq!(preview.first().unwrap().name, "Ch 0");
        assert_eq!(preview.last().unwrap().name, "Ch 10");
        assert_eq!(
            preview.last().unwrap().transitions.last().unwrap().0,
            59_999_000.0
        );
        let compiled = compiler
            .lower(widget.graph())
            .expect("wasm demo should lower cleanly");
        // Presentation choices are host-owned and are therefore absent from
        // this processing-topology fixture.
        assert_eq!(widget.graph().nodes.len(), 9);
        assert_eq!(compiled.nodes.len(), 8);
    }

    #[test]
    fn auxiliary_test_graph_fixtures_build_with_registered_nodes() {
        let mut live_binary = NodeGraphWidget::new(build_node_registry());
        let source = test_graphs_tests::build_live_binary_test(&mut live_binary);
        assert!(live_binary.graph().nodes.contains_key(&source));
        assert_eq!(live_binary.graph().nodes.len(), 2);

        let mut uart = NodeGraphWidget::new(build_node_registry());
        test_graphs_tests::populate_uart_demo(&mut uart);
        assert_eq!(uart.graph().nodes.len(), 3);
        assert_eq!(uart.graph().connections.len(), 3);
    }

    #[test]
    fn graph_file_api_round_trips_the_startup_graph() {
        let mut original = NodeGraphWidget::new(build_node_registry());
        test_graphs_tests::populate_startup(&mut original);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pipeline.json");
        original.save_to_path(&path).unwrap();

        let mut loaded = NodeGraphWidget::new(build_node_registry());
        loaded.load_from_path(&path).unwrap();

        assert_eq!(loaded.graph().nodes.len(), 10);
        assert_eq!(loaded.graph().connections.len(), 23);
    }

    /// Save/load round-trip: serializing the graph to JSON and
    /// restoring it through the registry (the File > Save/Load path) must
    /// compile to the same pipeline.
    #[test]
    fn graph_json_round_trip_compiles_identically() {
        let mut widget = NodeGraphWidget::new(build_node_registry());
        test_graphs_tests::populate_startup(&mut widget);
        let compiler = GraphCompiler::new();
        let original = compiler.lower(widget.graph()).expect("original lowers");

        let json = serde_json::to_string(widget.graph()).expect("graph serializes");
        let restored_state: node_graph::GraphState =
            serde_json::from_str(&json).expect("graph deserializes");
        let mut restored = NodeGraphWidget::new(build_node_registry());
        restored.set_graph(restored_state);

        let reloaded = compiler.lower(restored.graph()).expect("restored lowers");

        assert_eq!(original.nodes.len(), reloaded.nodes.len());
        for (a, b) in original.nodes.iter().zip(&reloaded.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.builder, b.builder);
            assert_eq!(
                a.state, b.state,
                "state of {} changed in round-trip",
                a.builder
            );
        }
        let edges = |compiled: &CompiledGraph| {
            let mut edges: Vec<String> = compiled
                .edges
                .iter()
                .map(|edge| {
                    format!(
                        "n{}:{} -> n{}:{} ({})",
                        edge.from.0.0, edge.from.1, edge.to.0.0, edge.to.1, edge.buffer
                    )
                })
                .collect();
            edges.sort();
            edges
        };
        assert_eq!(edges(&original), edges(&reloaded));
    }
}
