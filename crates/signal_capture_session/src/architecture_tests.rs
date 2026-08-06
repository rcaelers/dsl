fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn capture_sessions_do_not_leak_provider_application_or_lower_owner_details() {
    // Cargo metadata proves dependency direction, but it cannot detect name-based provider cases,
    // application-selected namespaces, or public re-exports, so these remain source assertions.
    let sources = [
        include_str!("live_capture/implementation.rs"),
        include_str!("live_capture_store/mod.rs"),
        include_str!("live_capture_store/artifact_store.rs"),
    ];
    for token in ["U3Pro16", "u3pro16"] {
        assert!(
            sources
                .iter()
                .all(|source| !implementation_source(source).contains(token)),
            "generic capture-session storage contains provider token {token:?}"
        );
    }

    let source = include_str!("live_capture_store/session_repository.rs");
    for token in ["default_capture_session_directory", ".join(\"dsl\")"] {
        assert!(
            !source.contains(token),
            "generic session storage contains application cache policy {token:?}"
        );
    }

    let library = include_str!("lib.rs");
    for forbidden in [
        "pub use platform_artifacts",
        "pub use platform_runtime",
        "pub use signal_capture",
        "pub use signal_derived",
        "pub use signal_runtime",
        "mod capture;",
        "mod derived_data_collector;",
        "mod derived_word_store;",
        "mod runtime;",
        "mod waveform_index;",
    ] {
        assert!(
            !library.contains(forbidden),
            "signal_capture_session redirects lower owner {forbidden:?}"
        );
    }
}
