fn source(relative_path: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(relative_path),
    )
    .unwrap_or_else(|error| panic!("could not read {relative_path}: {error}"))
}

#[test]
fn application_shell_consumes_the_provider_contract_without_source_kind_branches() {
    let app = source("app.rs");
    let hooks = source("app_platform/hooks.rs");

    for source in [&app, &hooks] {
        for forbidden in [
            "SourceDataKind::File",
            "SourceDataKind::Live",
            "SourcePreparationStatus",
            "SourcePreparationUpdate",
            "PreparedCaptureData",
            "publish_file_source",
            "publish_live_source",
        ] {
            assert!(
                !source.contains(forbidden),
                "application shell bypasses the capture-provider boundary with {forbidden}"
            );
        }
    }

    assert!(app.contains("apply_capture_provider_poll"));
    assert!(hooks.contains("PreparedCaptureProvider::new"));
    assert!(app.contains("LiveCaptureProvider::new"));
}

#[test]
fn acquisition_is_an_optional_provider_capability() {
    let contract = source("capture_provider/contract.rs");
    let live = source("capture_provider/live.rs");
    let prepared = source("capture_provider/prepared.rs");

    assert!(
        contract
            .contains("fn acquisition(&mut self) -> Option<&mut dyn CaptureCoordinatorContract>")
    );
    assert!(live.contains("fn acquisition(&mut self)"));
    assert!(!prepared.contains("fn acquisition(&mut self)"));
}

#[test]
fn provider_adapters_share_one_presentation_and_readiness_contract() {
    let live = source("capture_provider/live.rs");
    let prepared = source("capture_provider/prepared.rs");

    for adapter in [&live, &prepared] {
        assert!(adapter.contains("impl CaptureDataProvider for"));
        assert!(adapter.contains("CaptureProviderPoll"));
        assert!(adapter.contains("CapturePresentationUpdate"));
        assert!(adapter.contains("CaptureReadinessUpdate"));
    }
}

#[test]
fn presentation_identity_is_owned_by_the_capture_lifecycle() {
    let lifecycle = source("capture_analysis_lifecycle/state.rs");
    let platform = source("app_platform/state.rs");

    assert!(lifecycle.contains("presentation_identity: Option<String>"));
    assert!(!platform.contains("capture_presentation_identity"));
}
