#[test]
fn application_orchestration_depends_on_the_ui_owned_graph_service() {
    for (name, source) in [
        ("application", include_str!("../app.rs")),
        (
            "native platform hooks",
            include_str!("../app_platform/native_hooks.rs"),
        ),
        (
            "wasm platform hooks",
            include_str!("../app_platform/wasm_hooks.rs"),
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
