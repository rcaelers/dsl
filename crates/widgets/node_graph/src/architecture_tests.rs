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
