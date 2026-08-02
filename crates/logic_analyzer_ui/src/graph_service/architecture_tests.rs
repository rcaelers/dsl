#[test]
fn application_orchestration_depends_on_the_ui_owned_graph_service() {
    for (name, source) in [
        ("application", include_str!("../app.rs")),
        (
            "application hooks",
            include_str!("../app_platform/hooks.rs"),
        ),
        (
            "capture coordinator",
            include_str!("../live_capture/implementation.rs"),
        ),
    ] {
        for concrete_type in ["GraphCompiler", "LiveRun"] {
            assert!(
                !source.contains(concrete_type),
                "{name} must depend on UI-owned graph ports rather than {concrete_type}"
            );
        }
    }

    let application = include_str!("../app.rs");
    assert!(application.contains("Box<dyn GraphService>"));
    assert!(application.contains("Option<Box<dyn GraphRun>>"));
    assert!(include_str!("../app_services.rs").contains("standard_graph_service()"));
}

#[test]
fn concrete_compiler_knowledge_is_confined_to_service_adapters() {
    let adapter = include_str!("graph_compiler.rs");

    assert!(adapter.contains("impl GraphService for GraphCompiler"));
    assert!(adapter.contains("impl GraphRun for LiveRun"));
    assert!(adapter.contains("Box::new(GraphCompiler::new())"));
}

#[test]
fn graph_service_uses_one_contract_and_adapter_on_every_target() {
    let module = include_str!("mod.rs");
    let contract = include_str!("contract.rs");

    assert!(!module.contains("target_arch"));
    assert!(!module.contains("platform_contract"));
    assert!(!module.contains("platform_graph_compiler"));
    assert!(contract.contains("fn derived_cache_configs_by_node("));
    assert!(contract.contains("fn clear_derived_cache_entry("));
    assert!(contract.contains("fn inspect_derived_cache_entry("));
}
