#[test]
fn capture_coordinator_depends_on_the_ui_owned_export_service() {
    let coordinator = include_str!("../live_capture/native.rs");

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
    let contract = include_str!("contract.rs");

    assert!(!module.contains("platform_contract"));
    assert!(!contract.contains("target_arch"));
    assert!(contract.contains("pub trait CaptureExportService"));
    assert!(contract.contains("fn start("));
    assert!(contract.contains("fn take_completion("));
}
