fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_compiler_has_no_concrete_node_or_protocol_special_cases() {
    // Dependency checks cannot detect branching on a persisted name or protocol string, so this
    // remains an intentional source-level assertion over the generic compiler implementation.
    let sources = [
        ("graph lowering", include_str!("graph.rs")),
        ("data collector", include_str!("data_collector.rs")),
        ("compiler facade", include_str!("graph_lowerer.rs")),
    ];
    let forbidden = [
        "CollectedDataKind",
        "CollectedValueKind",
        "DerivedLaneData",
        "org.logicconduit.digital-sample",
        "org.logicconduit.word",
        "org.logicconduit.trigger",
        "org.logicconduit.number-sample",
        "org.logicconduit.text-sample",
        "SPI Decoder",
        "Binary Decoder",
        "UART Decoder",
        "DeterministicFake",
        "BufferedFake",
        "U3Pro16",
        "u3pro16",
        "Sigrok",
        "sigrok",
        "Python decoder",
        "ViewerOutputSelection",
        "viewer_output_selections",
        "set_viewer_output_selected",
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic compiler {component} contains concrete domain token {token:?}"
            );
        }
    }
}
