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
