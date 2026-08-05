fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn capture_has_no_session_derived_graph_or_ui_dependency() {
    let sources = [
        include_str!("capture/implementation.rs"),
        include_str!("capture/query.rs"),
        include_str!("capture/worker_runtime.rs"),
        include_str!("capture/worker_replay_source.rs"),
        include_str!("capture_index_kernel.rs"),
        include_str!("edge_query.rs"),
        include_str!("recorded_edge_query.rs"),
        include_str!("sample.rs"),
        include_str!("waveform_index/builder.rs"),
        include_str!("waveform_index/reader.rs"),
        include_str!("waveform_index/storage.rs"),
    ];
    for forbidden in [
        "signal_capture_session",
        "live_capture",
        "CaptureSession",
        "CaptureStore",
        "DerivedLanes",
        "PayloadRegistry",
        "node_graph",
        "egui",
    ] {
        assert!(
            sources
                .iter()
                .all(|source| !implementation_source(source).contains(forbidden)),
            "capture owner contains unrelated dependency {forbidden:?}"
        );
    }
}

#[test]
fn capture_manifest_depends_only_on_lower_level_owners() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "signal-capture-session",
        "logic-analyzer-",
        "node-graph",
        "egui",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "capture manifest contains unrelated dependency {forbidden:?}"
        );
    }
}
