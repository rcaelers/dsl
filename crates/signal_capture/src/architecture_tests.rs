fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_capture_owner_has_no_session_derived_or_application_special_cases() {
    // Cargo metadata proves that higher-level owners are not dependencies, but it cannot detect
    // branching on their type names, so this remains an intentional source-level assertion.
    let sources = [
        include_str!("capture/contracts.rs"),
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
        "live_capture",
        "CaptureSession",
        "CaptureStore",
        "DerivedLanes",
        "PayloadRegistry",
    ] {
        assert!(
            sources
                .iter()
                .all(|source| !implementation_source(source).contains(forbidden)),
            "generic capture owner contains higher-level token {forbidden:?}"
        );
    }
}
