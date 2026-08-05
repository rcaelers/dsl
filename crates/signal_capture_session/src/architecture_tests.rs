fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_capture_storage_contains_no_concrete_provider_contracts() {
    let sources = [
        include_str!("live_capture/implementation.rs"),
        include_str!("live_capture_store/mod.rs"),
        include_str!("live_capture_store/artifact_store.rs"),
    ];
    for token in ["DeterministicFake", "BufferedFake", "U3Pro16", "u3pro16"] {
        assert!(
            sources
                .iter()
                .all(|source| !implementation_source(source).contains(token)),
            "generic capture-session storage contains provider token {token:?}"
        );
    }
}

#[test]
fn generic_storage_does_not_choose_an_application_cache_namespace() {
    let source = include_str!("live_capture_store/session_repository.rs");
    for token in ["default_capture_session_directory", ".join(\"dsl\")"] {
        assert!(
            !source.contains(token),
            "generic session storage contains application cache policy {token:?}"
        );
    }
}

#[test]
fn lower_level_contracts_are_not_redirected_through_signal_capture_session() {
    let library = include_str!("lib.rs");
    for forbidden in [
        "pub use signal_artifacts",
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
