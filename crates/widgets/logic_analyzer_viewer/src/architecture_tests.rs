fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_viewer_has_no_domain_specific_or_storage_owned_fallbacks() {
    // The dependency graph cannot detect branching on protocol labels or reaching through an
    // allowed generic data crate to obsolete/storage-owning APIs, so this remains textual.
    let sources = [
        ("channel", include_str!("channel.rs"), true),
        ("cursor", include_str!("cursor.rs"), true),
        ("derived drawing", include_str!("draw/derived.rs"), true),
        ("frame drawing", include_str!("draw/frame.rs"), true),
        ("drawing facade", include_str!("draw/mod.rs"), true),
        ("lanes", include_str!("lanes.rs"), true),
        ("crate facade", include_str!("lib.rs"), false),
        ("viewer", include_str!("viewer.rs"), true),
    ];
    let forbidden = [
        "ArtifactRepository",
        "BufferedFake",
        "CaptureDataSource",
        "CollectedValueKind",
        "DecoderTable",
        "DerivedLaneData",
        "DeterministicFake",
        "LaneSummary",
        "MemoryArtifactRepository",
        "MemorySnapshot",
        "Python decoder",
        "Sigrok",
        "StorageSnapshot",
        "UART",
        "U3Pro16",
        "ViewerTable",
        "memory_snapshot",
        "set_capture_path",
        "sigrok",
        "std::fs",
        "storage_snapshot",
        "u3pro16",
        "u64::MAX - 1",
        "u64::MAX - 2",
        "uart_",
        "uart_data_lane_name",
        "\"Bits\"",
        "\"Data\"",
    ];

    for (component, source, strip_tests) in sources {
        let source = if strip_tests {
            implementation_source(source)
        } else {
            source
        };
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic viewer {component} contains domain/storage token {token:?}"
            );
        }
    }
}
