fn owner_service_is_active(
    service: Box<dyn logic_analyzer_capture_export::CaptureExportService>,
) -> bool {
    service.is_active()
}

#[test]
fn ui_capture_export_port_is_the_owner_contract() {
    let ui_service: Box<dyn logic_analyzer_ui::CaptureExportService> =
        logic_analyzer_ui::unavailable_capture_export_service();

    assert!(!owner_service_is_active(ui_service));
}
