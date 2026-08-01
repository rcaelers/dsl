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
        ("scheduler", include_str!("scheduler.rs")),
        ("edge queries", include_str!("edge_query.rs")),
        ("senders", include_str!("sender.rs")),
        ("ports", include_str!("ports.rs")),
        (
            "cooperative manager",
            include_str!("cooperative_manager.rs"),
        ),
        ("threaded manager", include_str!("manager.rs")),
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
            "native capture store",
            include_str!("live_capture_store/native.rs"),
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
        include_str!("live_capture_store/repository_native.rs"),
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
fn prepared_byte_sources_are_portable_storage_contracts() {
    let contract = include_str!("storage/contract.rs");
    let memory = include_str!("storage/memory.rs");

    for forbidden in ["target_arch", "PathBuf", "std::fs", "memmap", "web_sys"] {
        assert!(
            !contract.contains(forbidden) && !memory.contains(forbidden),
            "portable prepared-byte storage contains host detail {forbidden:?}"
        );
    }
    for required in [
        "trait RandomAccessReader",
        "trait PreparedByteSource",
        "trait ImmutableByteRegion",
        "struct SourceIdentity",
        "struct ByteRange",
    ] {
        assert!(
            contract.contains(required),
            "missing storage contract {required}"
        );
    }
}

#[test]
fn derived_word_encoding_and_presence_are_shared_between_targets() {
    let module = include_str!("derived_word_store/mod.rs");
    let codec = include_str!("derived_word_store/codec.rs");
    let format = include_str!("derived_word_store/format.rs");
    let presence = include_str!("derived_word_store/presence.rs");
    let vlq = include_str!("derived_word_store/vlq.rs");
    let integrity = include_str!("crc32c.rs");

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
    ] {
        assert!(
            !implementation_source(source).contains("target_arch"),
            "{component} must not select a target-specific implementation"
        );
    }
}

#[test]
fn wasm_derived_store_keeps_committed_words_in_encoded_blocks() {
    let wasm_store = include_str!("derived_word_store/store_wasm.rs");

    for required in [
        "struct EncodedBlock",
        "blocks: Vec<EncodedBlock>",
        "WordBlockBuilder",
        "decode_word_block",
        "word_presence_summaries",
    ] {
        assert!(
            wasm_store.contains(required),
            "the wasm derived-word store is missing encoded-block component {required}"
        );
    }
    assert!(
        !wasm_store.contains("words: Vec<Word>"),
        "the wasm derived-word store must not retain an authoritative word vector"
    );
}

#[test]
fn application_manager_is_a_facade_instead_of_a_target_dependent_alias() {
    let library = include_str!("lib.rs");
    assert!(!library.contains("type AppManager"));

    for implementation in [
        include_str!("app_manager/native.rs"),
        include_str!("app_manager/wasm.rs"),
    ] {
        assert!(implementation.contains("pub struct AppManager"));
        for operation in [
            "add_node_deferred",
            "start_all_deferred",
            "reconfigure_at",
            "restart_node",
            "request_stop",
            "pump",
        ] {
            assert!(
                implementation.contains(&format!("fn {operation}")),
                "AppManager backend is missing {operation}"
            );
        }
    }
}
