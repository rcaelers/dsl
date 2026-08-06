#[test]
fn capture_coordinator_depends_on_the_export_owner_service() {
    let coordinator = include_str!("../live_capture/coordinator.rs");

    assert!(coordinator.contains("Box<dyn CaptureExportService>"));
    assert!(coordinator.contains("self.export_service.start("));
    for concrete_export_detail in [
        "logic_analyzer_capture_export",
        "export_finalized_capture",
        "CaptureExportObserver",
        "ActiveExport",
        "capture-export",
    ] {
        assert!(
            !coordinator.contains(concrete_export_detail),
            "capture coordinator must not contain {concrete_export_detail}"
        );
    }
}

#[test]
fn export_contract_is_identical_on_every_target() {
    let module = include_str!("mod.rs");

    assert!(!module.contains("target_arch"));
    assert!(module.contains("pub use logic_analyzer_capture_export"));
}
