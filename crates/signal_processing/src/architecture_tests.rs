fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_runtime_contains_no_concrete_source_or_protocol_contracts() {
    let sources = [
        ("samples", include_str!("sample.rs")),
        ("scheduler", include_str!("runtime/scheduler.rs")),
        ("edge queries", include_str!("edge_query.rs")),
        ("senders", include_str!("runtime/sender.rs")),
        ("ports", include_str!("runtime/ports.rs")),
        (
            "cooperative manager",
            include_str!("runtime/cooperative_manager.rs"),
        ),
        ("threaded manager", include_str!("runtime/manager.rs")),
        ("events", include_str!("events.rs")),
        (
            "derived-data catalog",
            include_str!("derived_data_collector/catalog.rs"),
        ),
        (
            "derived-data collector",
            include_str!("derived_data_collector/collector.rs"),
        ),
        (
            "digital derived-data adapter",
            include_str!("derived_data_collector/digital.rs"),
        ),
        (
            "number derived-data adapter",
            include_str!("derived_data_collector/number.rs"),
        ),
        (
            "text derived-data adapter",
            include_str!("derived_data_collector/text.rs"),
        ),
        (
            "trigger derived-data adapter",
            include_str!("derived_data_collector/trigger.rs"),
        ),
        (
            "word derived-data adapter",
            include_str!("derived_data_collector/word.rs"),
        ),
    ];
    let forbidden = [
        "DslFileSource",
        "LogicAnalyzerSource",
        "DSLogic",
        "U3Pro16",
        "Binary Decoder",
        "SPI",
        "UART",
        "I2C",
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic {component} source contains concrete token {token:?}"
            );
        }
    }
}

#[test]
fn generic_capture_storage_contains_no_concrete_provider_contracts() {
    let sources = [
        (
            "capture runtime",
            include_str!("live_capture/implementation.rs"),
        ),
        ("capture store", include_str!("live_capture_store/mod.rs")),
        (
            "artifact capture store",
            include_str!("live_capture_store/artifact_store.rs"),
        ),
        ("waveform index", include_str!("waveform_index/mod.rs")),
    ];
    for (component, source) in sources {
        let source = implementation_source(source);
        for token in ["DeterministicFake", "BufferedFake", "U3Pro16", "u3pro16"] {
            assert!(
                !source.contains(token),
                "generic {component} contains concrete capture-provider token {token:?}"
            );
        }
    }
}

#[test]
fn type_erased_collection_contract_has_no_builtin_payload_checks() {
    let source = implementation_source(include_str!("payload.rs"));
    for token in [
        "CollectedDataKind",
        "CollectedValueKind",
        "DerivedLaneData",
        "org.logicconduit.",
        "SPI",
        "UART",
        "Binary Decoder",
    ] {
        assert!(
            !source.contains(token),
            "generic payload contract contains built-in token {token:?}"
        );
    }
}

#[test]
fn generic_storage_does_not_choose_an_application_cache_namespace() {
    let sources = [
        include_str!("derived_word_store/persistent.rs"),
        include_str!("live_capture_store/session_repository.rs"),
    ];
    for token in [
        "default_cache_directory",
        "default_capture_session_directory",
        ".join(\"dsl\")",
    ] {
        assert!(
            sources.iter().all(|source| !source.contains(token)),
            "generic storage source contains application cache policy {token:?}"
        );
    }
}

#[test]
fn artifact_contracts_are_not_redirected_through_signal_processing() {
    let facade = include_str!("lib.rs");
    assert!(!facade.contains("pub use signal_artifacts"));
    assert!(!facade.contains("mod storage"));
}

#[test]
fn stream_execution_has_one_private_owner_facade() {
    let library = include_str!("lib.rs");
    let facade = include_str!("runtime/mod.rs");

    assert!(library.contains("mod runtime;"));
    for former_root_leaf in [
        "app_manager",
        "cooperative_manager",
        "errors",
        "graph",
        "manager",
        "node",
        "pipeline",
        "ports",
        "protocol",
        "receiver",
        "scheduler",
        "sender",
        "type_registry",
        "watchdog",
        "work_executor",
        "worker_operation_queue",
    ] {
        assert!(
            facade.contains(&format!("mod {former_root_leaf};")),
            "runtime owner does not declare {former_root_leaf}"
        );
        assert!(
            !library.contains(&format!("mod {former_root_leaf};")),
            "runtime leaf {former_root_leaf} escaped back to the crate root"
        );
    }
}

#[test]
fn stream_execution_does_not_depend_on_storage_or_session_owners() {
    let runtime_sources = [
        include_str!("runtime/app_manager/contract.rs"),
        include_str!("runtime/app_manager/cooperative.rs"),
        include_str!("runtime/app_manager/implementation.rs"),
        include_str!("runtime/cooperative_manager.rs"),
        include_str!("runtime/errors.rs"),
        include_str!("runtime/graph.rs"),
        include_str!("runtime/manager.rs"),
        include_str!("runtime/node.rs"),
        include_str!("runtime/pipeline.rs"),
        include_str!("runtime/ports.rs"),
        include_str!("runtime/protocol.rs"),
        include_str!("runtime/receiver.rs"),
        include_str!("runtime/scheduler.rs"),
        include_str!("runtime/sender.rs"),
        include_str!("runtime/type_registry.rs"),
        include_str!("runtime/watchdog.rs"),
        include_str!("runtime/work_executor.rs"),
        include_str!("runtime/worker_operation_queue.rs"),
    ];

    for forbidden_owner in [
        "crate::advanced_trigger",
        "crate::capture_policy",
        "crate::derived_data_collector",
        "crate::derived_word_store",
        "crate::live_capture",
        "crate::live_capture_store",
        "crate::logic_analyzer",
        "crate::payload",
        "crate::sampling_points",
        "crate::waveform_index",
        "signal_artifacts",
    ] {
        assert!(
            runtime_sources
                .iter()
                .all(|source| !implementation_source(source).contains(forbidden_owner)),
            "stream execution depends on unrelated owner {forbidden_owner}"
        );
    }
}

#[test]
fn derived_word_core_is_shared_between_targets() {
    let module = include_str!("derived_word_store/mod.rs");
    let codec = include_str!("derived_word_store/codec.rs");
    let format = include_str!("derived_word_store/format.rs");
    let presence = include_str!("derived_word_store/presence.rs");
    let vlq = include_str!("derived_word_store/vlq.rs");
    let integrity = include_str!("crc32c.rs");
    let cache = include_str!("derived_word_store/cache.rs");

    for shared_module in ["codec", "format", "vlq"] {
        assert!(
            module.contains(&format!("mod {shared_module};")),
            "the shared derived-word module {shared_module} is missing"
        );
        assert!(
            !module.contains(&format!(
                "#[cfg(not(target_arch = \"wasm32\"))]\nmod {shared_module};"
            )),
            "the derived-word module {shared_module} must not be native-only"
        );
    }

    for (component, source) in [
        ("derived-word codec", codec),
        ("derived-word format", format),
        ("derived-word presence index", presence),
        ("derived-word VLQ", vlq),
        ("derived-word integrity", integrity),
        ("derived-word decoded-block cache", cache),
    ] {
        assert!(
            !implementation_source(source).contains("target_arch"),
            "{component} must not select a target-specific implementation"
        );
    }
}

#[test]
fn derived_store_has_one_repository_backed_implementation() {
    let module = include_str!("derived_word_store/mod.rs");
    let store = include_str!("derived_word_store/store.rs");

    assert!(module.contains("mod store;"));
    assert!(!module.contains("target_arch"));
    assert!(!module.contains("mod platform;"));
    assert!(store.contains("ArtifactRepository"));
    assert!(store.contains("read_artifact_region"));
    assert!(!implementation_source(store).contains("target_arch"));
    assert!(!store.contains("std::fs"));
}

#[test]
fn application_manager_is_a_facade_instead_of_a_target_dependent_alias() {
    let library = include_str!("lib.rs");
    assert!(!library.contains("type AppManager"));
    let facade = include_str!("runtime/app_manager/mod.rs");
    assert!(!facade.contains("target_arch"));
    assert!(facade.contains("mod contract;"));
    assert!(facade.contains("mod cooperative;"));
    assert!(facade.contains("mod implementation;"));
    assert!(facade.contains("AppManagerBackend"));
    assert!(facade.contains("AppManagerFactory"));
}
