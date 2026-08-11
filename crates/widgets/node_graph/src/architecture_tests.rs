fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn node_definition_and_registry_contain_no_decoder_host_special_cases() {
    // Dependency metadata cannot detect name-based cases within otherwise generic definitions and
    // registry code, so this remains a deliberately narrow source assertion.
    let sources = [
        ("definition API", include_str!("api/node.rs")),
        ("registry", include_str!("runtime/registry.rs")),
    ];
    for (component, source) in sources {
        let source = implementation_source(source);
        for token in ["Sigrok", "sigrok", "Python decoder"] {
            assert!(
                !source.contains(token),
                "generic node-graph {component} contains decoder-host token {token:?}"
            );
        }
    }
}

#[test]
fn add_menu_ordering_contains_no_concrete_category_special_cases() {
    let source = include_str!("widget/graph/menu.rs");
    let production = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(before, _)| before);

    assert!(!production.contains("External Sigrok"));
}

#[test]
fn crate_root_exposes_only_editor_composition_not_api_or_implementation_facades() {
    let root = include_str!("lib.rs");

    for facade in ["api", "model", "runtime"] {
        assert!(
            !root.contains(&format!("pub use {facade}::")),
            "node-graph crate root duplicates the canonical {facade} import path"
        );
    }
    assert!(root.contains("pub use widget::{"));
}
