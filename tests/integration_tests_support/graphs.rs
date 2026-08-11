use egui::Pos2;

use node_graph::NodeGraphWidget;
use node_graph::api::{NodeId, SocketDirection, SocketId};

use super::catalog::node_name;

fn output_index(widget: &NodeGraphWidget, node: NodeId, name: &str) -> usize {
    widget.graph().nodes[&node]
        .outputs
        .iter()
        .position(|socket| socket.name == name)
        .unwrap_or_else(|| panic!("no output socket '{name}'"))
}

fn input_index(widget: &NodeGraphWidget, node: NodeId, name: &str) -> usize {
    widget.graph().nodes[&node]
        .inputs
        .iter()
        .position(|socket| socket.name == name && socket.visible)
        .unwrap_or_else(|| panic!("no input socket '{name}'"))
}

fn connect(widget: &mut NodeGraphWidget, from: (NodeId, &str), to: (NodeId, &str)) {
    let from_index = output_index(widget, from.0, from.1);
    let to_index = input_index(widget, to.0, to.1);
    widget.graph_mut().add_connection(
        SocketId {
            node: from.0,
            index: from_index,
            direction: SocketDirection::Output,
        },
        SocketId {
            node: to.0,
            index: to_index,
            direction: SocketDirection::Input,
        },
    );
}

fn add(widget: &mut NodeGraphWidget, stable_id: &str, x: f32, y: f32) -> NodeId {
    let name = node_name(stable_id);
    widget
        .add_node_at(name, Pos2::new(x, y))
        .unwrap_or_else(|| panic!("unknown node type '{name}'"))
}

pub(crate) fn build_binary_decoder_demo(widget: &mut NodeGraphWidget) {
    let source = add(
        widget,
        "org.logicconduit.graph-node.sources.sigrok-file-source/v1",
        40.0,
        300.0,
    );
    let mut source_state = widget.graph().nodes[&source].state.clone();
    source_state["channel_names"] = serde_json::Value::Array(
        (0..11)
            .map(|channel| serde_json::Value::String(format!("Ch {channel}")))
            .collect(),
    );
    source_state["demo_data"] = true.into();
    widget.set_node_state(source, source_state);
    let source_name = node_name("org.logicconduit.graph-node.sources.sigrok-file-source/v1");
    widget.graph_mut().nodes.get_mut(&source).unwrap().title = source_name.into();

    let spi = add(
        widget,
        "org.logicconduit.graph-node.decoders.spi-decoder/v1",
        360.0,
        80.0,
    );
    let start = add(
        widget,
        "org.logicconduit.graph-node.logic.word-matcher/v1",
        680.0,
        40.0,
    );
    let stop = add(
        widget,
        "org.logicconduit.graph-node.logic.word-matcher/v1",
        680.0,
        230.0,
    );
    let counter = add(
        widget,
        "org.logicconduit.graph-node.logic.counter/v1",
        960.0,
        40.0,
    );
    let latch = add(
        widget,
        "org.logicconduit.graph-node.logic.sr-flip-flop/v1",
        960.0,
        230.0,
    );
    let formatter = add(
        widget,
        "org.logicconduit.graph-node.logic.string-formatter/v1",
        1240.0,
        40.0,
    );
    let gate = add(
        widget,
        "org.logicconduit.graph-node.logic.logic-gate/v1",
        1198.4297,
        592.2656,
    );
    let decoder = add(
        widget,
        "org.logicconduit.graph-node.decoders.parallel-decoder/v1",
        1520.0,
        300.0,
    );

    let matcher_state = |widget: &NodeGraphWidget, node: NodeId, pattern: &str| {
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
    connect(widget, (source, "Ch 10"), (decoder, "Strobe"));
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

pub(crate) fn build_live_binary_test(widget: &mut NodeGraphWidget) -> NodeId {
    let source = add(
        widget,
        "org.logicconduit.graph-node.sources.dslogic-u3pro16/v1",
        40.0,
        80.0,
    );
    let mut source_state = widget.graph().nodes[&source].state.clone();
    source_state["mode"]["value"] = "Stream".into();
    source_state["sample_rate"]["value"] = "1 MHz".into();
    let enabled = source_state["channels"]["enabled"]
        .as_array_mut()
        .expect("hardware source channels are an array");
    enabled.fill(serde_json::Value::Bool(false));
    enabled[0] = true.into();
    enabled[1] = true.into();
    widget.set_node_state(source, source_state);

    let decoder = add(
        widget,
        "org.logicconduit.graph-node.decoders.parallel-decoder/v1",
        360.0,
        80.0,
    );
    let mut decoder_state = widget.graph().nodes[&decoder].state.clone();
    decoder_state["input_strategy"]["value"] = "Packed stream".into();
    widget.set_node_state(decoder, decoder_state);
    connect(widget, (source, "Ch 0"), (decoder, "Strobe"));
    connect(widget, (source, "Ch 1"), (decoder, "D"));
    source
}

pub(crate) fn populate_startup(widget: &mut NodeGraphWidget) {
    let source = add(
        widget,
        "org.logicconduit.graph-node.sources.dsl-file-source/v1",
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
        "org.logicconduit.graph-node.decoders.spi-decoder/v1",
        330.0,
        120.0,
    );
    let start = add(
        widget,
        "org.logicconduit.graph-node.logic.word-matcher/v1",
        620.0,
        40.0,
    );
    let stop = add(
        widget,
        "org.logicconduit.graph-node.logic.word-matcher/v1",
        620.0,
        230.0,
    );
    let counter = add(
        widget,
        "org.logicconduit.graph-node.logic.counter/v1",
        900.0,
        40.0,
    );
    let latch = add(
        widget,
        "org.logicconduit.graph-node.logic.sr-flip-flop/v1",
        900.0,
        230.0,
    );
    let formatter = add(
        widget,
        "org.logicconduit.graph-node.logic.string-formatter/v1",
        1160.0,
        40.0,
    );
    let gate = add(
        widget,
        "org.logicconduit.graph-node.logic.logic-gate/v1",
        1160.0,
        400.0,
    );
    let decoder = add(
        widget,
        "org.logicconduit.graph-node.decoders.parallel-decoder/v1",
        1440.0,
        260.0,
    );
    let writer = add(
        widget,
        "org.logicconduit.graph-node.sinks.file-writer/v1",
        1760.0,
        120.0,
    );

    let mut spi_state = widget.graph().nodes[&spi].state.clone();
    spi_state["word_size"]["value"] = 24.into();
    spi_state["has_miso"]["value"] = false.into();
    widget.set_node_state(spi, spi_state);
    let matcher_state = |widget: &NodeGraphWidget, node: NodeId, pattern: &str| {
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

    for (id, title) in [
        (start, "Match Start"),
        (stop, "Match Stop"),
        (gate, "Enable Gate"),
    ] {
        widget.graph_mut().nodes.get_mut(&id).unwrap().title = title.to_owned();
    }

    connect(widget, (source, "Ch 7"), (spi, "CLK"));
    connect(widget, (source, "Ch 6"), (spi, "MOSI"));
    connect(widget, (source, "Ch 8"), (spi, "CS#"));
    connect(widget, (spi, "MOSI Words"), (start, "Words"));
    connect(widget, (spi, "MOSI Words"), (stop, "Words"));
    connect(widget, (start, "Match"), (latch, "Set"));
    connect(widget, (stop, "Match"), (latch, "Reset"));
    connect(widget, (start, "Match"), (counter, "Trigger"));
    connect(widget, (counter, "Count"), (formatter, "Value"));
    connect(widget, (formatter, "Text"), (writer, "Filename"));
    connect(widget, (source, "Ch 8"), (gate, "In"));
    connect(widget, (latch, "Q"), (gate, "In"));
    connect(widget, (gate, "Out"), (decoder, "Enable"));
    connect(widget, (source, "Ch 10"), (decoder, "Strobe"));
    for bit in 0..8 {
        connect(widget, (source, &format!("Ch {bit}")), (decoder, "D"));
    }
    connect(widget, (decoder, "Words"), (writer, "Data"));
}
