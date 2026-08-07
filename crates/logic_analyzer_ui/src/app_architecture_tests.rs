#[test]
fn app_composes_owned_state_instead_of_redeclaring_lifecycle_fields() {
    let source = include_str!("app.rs");
    let app = source
        .split_once("pub struct App {")
        .expect("App declaration")
        .1
        .split_once("\n}")
        .expect("App declaration end")
        .0;

    for owner in [
        "graph_run: GraphRunLifecycle",
        "capture_analysis: CaptureAnalysisLifecycle",
        "presentations: PresentationCatalogs",
        "timeline_markers: TimelineMarkerBindings",
    ] {
        assert!(app.contains(owner), "App is missing state owner {owner}");
    }

    for former_field in [
        "graph_service:",
        "derived_cache_clear_task:",
        "capture_availability:",
        "trigger_configuration:",
        "capture_graph:",
        "capture_analysis_error:",
        "capture_epoch_observed_graph:",
        "run:",
        "run_message:",
        "cached_preview_graph:",
        "running_graph_semantics:",
        "sampling_overlay_candidates:",
        "selected_sampling_overlays:",
        "viewer_lane_order:",
        "decoder_panels:",
        "plugin_panels:",
        "presented_derived_lanes:",
        "output_presentation_catalog:",
        "timeline_marker_owners:",
    ] {
        assert!(
            !app.lines()
                .any(|line| line.trim_start().starts_with(former_field)),
            "App redeclares owner field {former_field}"
        );
    }
}

#[test]
fn lifecycle_owner_fields_are_private() {
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (owner, type_name, relative_path) in [
        (
            "graph run",
            "GraphRunLifecycle",
            "graph_run_lifecycle/state.rs",
        ),
        (
            "capture analysis",
            "CaptureAnalysisLifecycle",
            "capture_analysis_lifecycle/state.rs",
        ),
        (
            "presentation catalogs",
            "PresentationCatalogs",
            "presentation_catalogs/state.rs",
        ),
        (
            "timeline marker bindings",
            "TimelineMarkerBindings",
            "timeline_marker_bindings/state.rs",
        ),
    ] {
        let source = std::fs::read_to_string(source_root.join(relative_path))
            .unwrap_or_else(|error| panic!("could not read {owner} owner: {error}"));
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
