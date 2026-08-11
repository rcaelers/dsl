fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]\nmod ")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn capture_coordinator_composes_distinct_state_owners() {
    let source = include_str!("coordinator.rs");
    let coordinator = source
        .split_once("pub(crate) struct CaptureCoordinator {")
        .expect("CaptureCoordinator declaration")
        .1
        .split_once("\n}")
        .expect("CaptureCoordinator declaration end")
        .0;

    for owner in [
        "acquisition: CaptureAcquisition",
        "publication: CapturePublication",
        "projection: CaptureStatusProjection",
    ] {
        assert!(
            coordinator.contains(owner),
            "CaptureCoordinator is missing state owner {owner}"
        );
    }

    for former_field in [
        "repository:",
        "recent_sessions:",
        "status:",
        "active:",
        "completed:",
        "retired:",
        "waveform_update:",
        "analysis_attachment:",
        "export_service:",
        "pending_configuration_epoch:",
        "configuration_epoch_preparation:",
        "configuration_epoch_resolutions:",
        "configuration_epoch_notice:",
        "state_history:",
        "work_executor:",
    ] {
        assert!(
            !coordinator
                .lines()
                .any(|line| line.trim_start().starts_with(former_field)),
            "CaptureCoordinator redeclares owner field {former_field}"
        );
    }
}

#[test]
fn live_capture_owner_fields_are_private() {
    for (owner, type_name, source) in [
        (
            "acquisition",
            "CaptureAcquisition",
            include_str!("acquisition_state.rs"),
        ),
        (
            "publication",
            "CapturePublication",
            include_str!("storage_publication.rs"),
        ),
        (
            "status projection",
            "CaptureStatusProjection",
            include_str!("status_projection.rs"),
        ),
    ] {
        let declaration = source
            .split_once(&format!("pub(crate) struct {type_name} {{"))
            .unwrap_or_else(|| panic!("{owner} declaration"))
            .1
            .split_once("\n}")
            .unwrap_or_else(|| panic!("{owner} declaration end"))
            .0;
        assert!(
            !declaration.contains("pub(crate) ") && !declaration.contains("pub "),
            "{owner} exposes mutable state instead of methods"
        );
    }
}

#[test]
fn generic_ui_capture_components_have_no_device_or_protocol_special_cases() {
    // Cargo metadata rejects concrete implementation dependencies, but it cannot detect branching
    // on device/protocol names or intra-crate platform reach-through, so this remains an intentional
    // source-level assertion.
    for (owner, source) in [
        ("acquisition", include_str!("acquisition_state.rs")),
        ("coordinator", include_str!("coordinator.rs")),
        ("publication", include_str!("storage_publication.rs")),
        ("status projection", include_str!("status_projection.rs")),
    ] {
        assert!(
            !implementation_source(source).contains("app_platform"),
            "capture {owner} must use injected services instead of application platform state"
        );
    }

    let sources = [
        ("application", include_str!("../app.rs")),
        ("acquisition", include_str!("acquisition_state.rs")),
        ("coordinator contract", include_str!("contract.rs")),
        ("capture coordinator", include_str!("coordinator.rs")),
        ("publication", include_str!("storage_publication.rs")),
        ("status projection", include_str!("status_projection.rs")),
    ];
    let forbidden = [
        "Binary Decoder",
        "I2C",
        "Python decoder",
        "SPI",
        "Sigrok",
        "UART",
        "U3Pro16",
        "u3pro16",
        "sigrok",
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic UI {component} source contains concrete token {token:?}"
            );
        }
    }
}
