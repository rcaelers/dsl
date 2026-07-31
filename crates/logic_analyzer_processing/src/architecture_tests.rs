#[test]
fn support_facade_is_crate_private() {
    let crate_root = include_str!("lib.rs");
    assert!(
        !crate_root
            .lines()
            .any(|line| line.trim() == "pub mod support;")
    );

    let facade = include_str!("support/mod.rs");
    assert!(facade.lines().all(|line| !line.trim().starts_with("pub ")));
}

#[test]
fn visible_support_modules_are_not_also_flattened() {
    for facade in [
        include_str!("support/mod.rs"),
        include_str!("support/sigrokdecode/mod.rs"),
    ] {
        assert_visible_modules_are_not_reexported(facade);
    }
}

#[test]
fn cross_platform_capture_nodes_are_not_excluded_from_the_processing_catalog() {
    let sources = include_str!("nodes/sources/mod.rs");
    for module in ["dsl_file", "dslogic_u3pro16", "sigrok_file"] {
        assert!(sources.contains(&format!("pub mod {module};")));
        assert!(!sources.contains(&format!(
            "#[cfg(not(target_arch = \"wasm32\"))]\npub mod {module};"
        )));
    }

    let sinks = include_str!("nodes/sinks/mod.rs");
    for module in ["binary_file_writer", "csv_word_writer", "text_file_writer"] {
        assert!(sinks.contains(&format!("pub mod {module};")));
        assert!(!sinks.contains(&format!(
            "#[cfg(not(target_arch = \"wasm32\"))]\npub mod {module};"
        )));
    }
}

#[test]
fn cross_platform_capture_facades_expose_neutral_factories() {
    let crate_root = include_str!("lib.rs");
    let value_types = include_str!("types/mod.rs");
    assert!(crate_root.contains("pub use process_node_construction::ProcessNodeConstruction;"));
    assert!(!value_types.contains("ProcessNodeConstruction"));

    for facade in [
        include_str!("nodes/sources/dsl_file/facade.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/facade.rs"),
        include_str!("nodes/sources/sigrok_file/facade.rs"),
        include_str!("nodes/sinks/binary_file_writer/facade.rs"),
        include_str!("nodes/sinks/csv_word_writer/facade.rs"),
        include_str!("nodes/sinks/text_file_writer/facade.rs"),
    ] {
        assert!(facade.contains("Factory: Send + Sync"));
        assert!(facade.contains("Result<ProcessNodeConstruction, String>"));
        assert!(!facade.contains("Box<dyn ProcessNode>"));
    }
}

#[test]
fn platform_modules_select_complete_factory_implementations() {
    for facade in [
        include_str!("nodes/sources/dsl_file/platform/mod.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/platform/mod.rs"),
        include_str!("nodes/sources/sigrok_file/platform/mod.rs"),
    ] {
        assert!(facade.contains("implementation::source_factory"));
        assert!(!facade.contains("create_source"));
    }
    for facade in [
        include_str!("nodes/sinks/binary_file_writer/platform/mod.rs"),
        include_str!("nodes/sinks/csv_word_writer/platform/mod.rs"),
        include_str!("nodes/sinks/text_file_writer/platform/mod.rs"),
    ] {
        assert!(facade.contains("implementation::writer_factory"));
        assert!(!facade.contains("create_writer"));
    }
}

fn assert_visible_modules_are_not_reexported(facade: &str) {
    for line in facade.lines().map(str::trim) {
        let module = line
            .strip_prefix("pub mod ")
            .or_else(|| line.strip_prefix("pub(crate) mod "))
            .and_then(|declaration| declaration.strip_suffix(';'));
        let Some(module) = module else {
            continue;
        };

        assert!(
            !facade.contains(&format!("use {module}::")),
            "visible module `{module}` must not also have its symbols re-exported"
        );
    }
}
