use std::collections::{BTreeSet, HashMap, HashSet};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

static WORKSPACE_METADATA: OnceLock<Value> = OnceLock::new();

fn workspace_metadata() -> &'static Value {
    WORKSPACE_METADATA.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args(["metadata", "--format-version", "1"])
            .output()
            .expect("cargo metadata must run");
        assert!(
            output.status.success(),
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("cargo metadata must be JSON")
    })
}

fn workspace_member_ids(metadata: &Value) -> HashSet<&str> {
    metadata["workspace_members"]
        .as_array()
        .expect("workspace members must be an array")
        .iter()
        .map(|member| {
            member
                .as_str()
                .expect("workspace member ID must be a string")
        })
        .collect()
}

fn package<'a>(metadata: &'a Value, name: &str) -> &'a Value {
    let workspace_member_ids = workspace_member_ids(metadata);
    metadata["packages"]
        .as_array()
        .expect("metadata packages must be an array")
        .iter()
        .find(|package| {
            package["name"] == name
                && workspace_member_ids
                    .contains(package["id"].as_str().expect("package ID must be a string"))
        })
        .unwrap_or_else(|| panic!("workspace must contain {name}"))
}

fn resolved_non_dev_dependencies(metadata: &Value, owner: &str) -> BTreeSet<String> {
    let package_names = metadata["packages"]
        .as_array()
        .expect("metadata packages must be an array")
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package ID must be a string"),
                package["name"]
                    .as_str()
                    .expect("package name must be a string"),
            )
        })
        .collect::<HashMap<_, _>>();
    let owner_id = package(metadata, owner)["id"]
        .as_str()
        .expect("package ID must be a string");
    let owner_node = metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolved metadata nodes must be an array")
        .iter()
        .find(|node| node["id"] == owner_id)
        .unwrap_or_else(|| panic!("resolved dependency graph must contain {owner}"));

    owner_node["deps"]
        .as_array()
        .expect("resolved dependencies must be an array")
        .iter()
        .filter(|dependency| {
            dependency["dep_kinds"]
                .as_array()
                .expect("resolved dependency kinds must be an array")
                .iter()
                .any(|kind| kind["kind"] != "dev")
        })
        .map(|dependency| {
            let package_id = dependency["pkg"]
                .as_str()
                .expect("resolved package ID must be a string");
            package_names
                .get(package_id)
                .unwrap_or_else(|| panic!("resolved package {package_id} must have metadata"))
                .to_string()
        })
        .collect()
}

fn local_resolved_non_dev_dependencies(metadata: &Value, owner: &str) -> BTreeSet<String> {
    let workspace_member_ids = workspace_member_ids(metadata);
    let workspace_package_names = metadata["packages"]
        .as_array()
        .expect("metadata packages must be an array")
        .iter()
        .filter(|candidate| {
            workspace_member_ids.contains(
                candidate["id"]
                    .as_str()
                    .expect("package ID must be a string"),
            )
        })
        .map(|candidate| {
            candidate["name"]
                .as_str()
                .expect("package name must be a string")
                .to_owned()
        })
        .collect::<HashSet<_>>();

    resolved_non_dev_dependencies(metadata, owner)
        .into_iter()
        .filter(|dependency| workspace_package_names.contains(dependency))
        .collect()
}

fn direct_dependencies(metadata: &Value, owner: &str) -> BTreeSet<String> {
    package(metadata, owner)["dependencies"]
        .as_array()
        .expect("package dependencies must be an array")
        .iter()
        .map(|dependency| {
            dependency["name"]
                .as_str()
                .expect("dependency name must be a string")
                .to_owned()
        })
        .collect()
}

fn assert_forbidden_edges(metadata: &Value, owner: &str, forbidden: &[&str]) {
    let dependencies = resolved_non_dev_dependencies(metadata, owner);
    let forbidden = forbidden
        .iter()
        .map(|dependency| (*dependency).to_owned())
        .collect::<BTreeSet<_>>();
    let violations = dependencies
        .intersection(&forbidden)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        violations.is_empty(),
        "{owner} has forbidden non-dev dependency edges: {violations:?}"
    );
}

fn assert_required_edges(metadata: &Value, owner: &str, required: &[&str]) {
    let dependencies = resolved_non_dev_dependencies(metadata, owner);
    let missing = required
        .iter()
        .filter(|dependency| !dependencies.contains(**dependency))
        .copied()
        .collect::<BTreeSet<_>>();
    assert!(
        missing.is_empty(),
        "{owner} is missing required non-dev dependency edges: {missing:?}"
    );
}

#[test]
fn platform_contract_crates_have_no_product_dependencies() {
    let metadata = workspace_metadata();

    for owner in ["platform-artifacts", "platform-runtime"] {
        let dependencies = local_resolved_non_dev_dependencies(metadata, owner);
        assert!(
            dependencies.is_empty(),
            "{owner} must remain independent of every workspace crate: {dependencies:?}"
        );
    }
}

#[test]
fn platform_depends_only_on_neutral_contract_owners() {
    let metadata = workspace_metadata();
    let dependencies = local_resolved_non_dev_dependencies(metadata, "platform");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "platform-artifacts".to_owned(),
            "platform-runtime".to_owned(),
        ]),
        "platform may depend only on its neutral contract owners"
    );
}

#[test]
fn capture_export_owner_does_not_depend_on_ui_or_platform() {
    assert_forbidden_edges(
        workspace_metadata(),
        "logic-analyzer-capture-export",
        &["logic-analyzer-ui", "platform"],
    );
}

#[test]
fn ui_has_no_host_adapter_concrete_node_or_shared_test_dependencies() {
    let dependencies = direct_dependencies(workspace_metadata(), "logic-analyzer-ui");
    let forbidden = BTreeSet::from([
        "logic-analyzer-graph-nodes".to_owned(),
        "logic-analyzer-test-support".to_owned(),
        "platform".to_owned(),
        "rfd".to_owned(),
    ]);
    let violations = dependencies
        .intersection(&forbidden)
        .cloned()
        .collect::<BTreeSet<_>>();

    assert!(
        violations.is_empty(),
        "UI code must use injected ports and UI-owned test fakes; host adapters and composition dependencies belong outside the UI crate: {violations:?}"
    );
}

#[test]
fn viewer_depends_only_on_generic_widget_and_data_contracts() {
    let dependencies =
        local_resolved_non_dev_dependencies(workspace_metadata(), "logic-analyzer-viewer");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "input-bindings".to_owned(),
            "platform-artifacts".to_owned(),
            "signal-capture".to_owned(),
            "signal-capture-session".to_owned(),
            "signal-derived".to_owned(),
        ]),
        "the reusable viewer may depend only on generic interaction, artifact, capture, and derived-data contracts"
    );
}

#[test]
fn derived_data_depends_only_on_generic_lower_level_contracts() {
    let dependencies = local_resolved_non_dev_dependencies(workspace_metadata(), "signal-derived");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "platform-artifacts".to_owned(),
            "platform-runtime".to_owned(),
            "signal-capture".to_owned(),
            "signal-runtime".to_owned(),
        ]),
        "generic derived-data infrastructure may depend only on artifact, execution, and capture contracts"
    );
}

#[test]
fn signal_runtime_depends_only_on_the_host_scheduling_contract() {
    let dependencies = local_resolved_non_dev_dependencies(workspace_metadata(), "signal-runtime");

    assert_eq!(
        dependencies,
        BTreeSet::from(["platform-runtime".to_owned()]),
        "generic typed-stream execution may depend only on the neutral host scheduling contract"
    );
}

#[test]
fn signal_capture_depends_only_on_generic_storage_and_execution_contracts() {
    let dependencies = local_resolved_non_dev_dependencies(workspace_metadata(), "signal-capture");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "platform-artifacts".to_owned(),
            "platform-runtime".to_owned(),
            "signal-runtime".to_owned(),
        ]),
        "generic immutable capture and indexing may depend only on storage and execution contracts"
    );
}

#[test]
fn capture_sessions_depend_only_on_generic_capture_and_storage_contracts() {
    let dependencies =
        local_resolved_non_dev_dependencies(workspace_metadata(), "signal-capture-session");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "platform-artifacts".to_owned(),
            "platform-runtime".to_owned(),
            "signal-capture".to_owned(),
            "signal-derived".to_owned(),
            "signal-runtime".to_owned(),
        ]),
        "generic capture-session infrastructure may depend only on storage, capture, derived-data, and execution contracts"
    );
}

#[test]
fn node_graph_depends_only_on_portable_widget_and_document_contracts() {
    let dependencies = local_resolved_non_dev_dependencies(workspace_metadata(), "node-graph");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "input-bindings".to_owned(),
            "node-graph-document".to_owned(),
            "widget-support".to_owned(),
        ]),
        "the generic node editor may depend only on portable input, document, and widget contracts"
    );
}

#[test]
fn capture_formats_depend_only_on_portable_capture_and_host_contracts() {
    let dependencies =
        local_resolved_non_dev_dependencies(workspace_metadata(), "logic-analyzer-capture-formats");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "platform-artifacts".to_owned(),
            "platform-runtime".to_owned(),
            "signal-capture".to_owned(),
            "signal-capture-session".to_owned(),
            "signal-generators".to_owned(),
            "signal-runtime".to_owned(),
        ]),
        "capture-format processing may depend only on portable storage, capture, source-generation, and execution contracts"
    );
}

#[test]
fn dslogic_device_depends_only_on_portable_device_runtime_contracts() {
    let dependencies =
        local_resolved_non_dev_dependencies(workspace_metadata(), "logic-analyzer-device-dslogic");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "platform-artifacts".to_owned(),
            "platform-runtime".to_owned(),
            "signal-capture".to_owned(),
            "signal-capture-session".to_owned(),
            "signal-runtime".to_owned(),
        ]),
        "DSLogic device behavior may depend only on portable storage, scheduling, capture, and typed-stream contracts"
    );
}

#[test]
fn signal_sinks_depend_only_on_portable_stream_and_data_contracts() {
    let dependencies = local_resolved_non_dev_dependencies(workspace_metadata(), "signal-sinks");

    assert_eq!(
        dependencies,
        BTreeSet::from([
            "signal-capture".to_owned(),
            "signal-derived".to_owned(),
            "signal-runtime".to_owned(),
        ]),
        "portable terminal consumers may depend only on capture, derived-data, and typed-stream contracts"
    );
}

#[test]
fn trigger_editor_depends_only_on_the_provider_neutral_trigger_contract() {
    let dependencies = local_resolved_non_dev_dependencies(workspace_metadata(), "trigger-editor");

    assert_eq!(
        dependencies,
        BTreeSet::from(["signal-capture-session".to_owned()]),
        "the generic trigger widget may depend only on the provider-neutral trigger contract"
    );
}

#[test]
fn headless_graph_tier_depends_on_the_document_model_not_the_node_editor() {
    let metadata = workspace_metadata();

    for owner in [
        "logic-analyzer-graph-capabilities",
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-orchestration",
        "logic-analyzer-graph-plan",
        "logic-analyzer-graph-registry",
        "logic-analyzer-graph-runtime",
    ] {
        assert_forbidden_edges(metadata, owner, &["node-graph"]);
    }

    for owner in [
        "logic-analyzer-graph-capabilities",
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-orchestration",
        "logic-analyzer-graph-plan",
        "logic-analyzer-graph-runtime",
    ] {
        let dependencies = resolved_non_dev_dependencies(metadata, owner);
        assert!(
            dependencies.contains("node-graph-document"),
            "{owner} must consume the neutral graph document contract"
        );
    }

    let document_dependencies =
        local_resolved_non_dev_dependencies(metadata, "node-graph-document");
    assert!(
        document_dependencies.is_empty(),
        "the graph document model must have no workspace dependencies: {document_dependencies:?}"
    );
}

#[test]
fn workspace_dependency_direction_has_no_forbidden_product_edges() {
    let metadata = workspace_metadata();
    let rules: &[(&str, &[&str])] = &[
        (
            "logic-analyzer-graph-capabilities",
            &[
                "egui",
                "logic-analyzer-graph-compiler",
                "logic-analyzer-graph-nodes",
                "logic-analyzer-graph-registry",
                "logic-analyzer-graph-runtime",
                "logic-analyzer-ui",
                "logic-analyzer-viewer",
                "node-graph",
            ],
        ),
        (
            "logic-analyzer-graph-compiler",
            &[
                "egui",
                "logic-analyzer-capture-formats",
                "logic-analyzer-device-dslogic",
                "logic-analyzer-graph-editor-registry",
                "logic-analyzer-graph-nodes",
                "logic-analyzer-graph-runtime",
                "logic-analyzer-protocol-decoders",
                "logic-analyzer-viewer",
                "node-graph",
                "signal-generators",
                "signal-sinks",
                "signal-transforms",
            ],
        ),
        (
            "logic-analyzer-graph-plan",
            &[
                "logic-analyzer-graph-compiler",
                "logic-analyzer-graph-registry",
                "logic-analyzer-graph-runtime",
                "logic-analyzer-ui",
                "node-graph",
            ],
        ),
        (
            "logic-analyzer-graph-runtime",
            &[
                "egui",
                "logic-analyzer-capture-formats",
                "logic-analyzer-device-dslogic",
                "logic-analyzer-graph-compiler",
                "logic-analyzer-graph-nodes",
                "logic-analyzer-graph-registry",
                "logic-analyzer-protocol-decoders",
                "logic-analyzer-ui",
                "logic-analyzer-viewer",
                "node-graph",
                "signal-generators",
                "signal-sinks",
                "signal-transforms",
            ],
        ),
        (
            "logic-analyzer-graph-registry",
            &[
                "egui",
                "logic-analyzer-graph-compiler",
                "logic-analyzer-graph-nodes",
                "logic-analyzer-graph-runtime",
                "logic-analyzer-ui",
                "node-graph",
                "platform",
            ],
        ),
        (
            "logic-analyzer-graph-nodes",
            &["logic-analyzer-graph-compiler"],
        ),
        ("example-plugin", &["logic-analyzer-graph-compiler"]),
        (
            "logic-analyzer-ui",
            &[
                "logic-analyzer-capture-formats",
                "logic-analyzer-device-dslogic",
                "logic-analyzer-graph-nodes",
                "logic-analyzer-protocol-decoders",
                "signal-generators",
                "signal-sinks",
                "signal-transforms",
            ],
        ),
        (
            "platform",
            &["logic-analyzer-graph-nodes", "logic-analyzer-ui"],
        ),
    ];

    for (owner, forbidden) in rules {
        assert_forbidden_edges(metadata, owner, forbidden);
    }

    assert_required_edges(
        metadata,
        "logic-analyzer-graph-orchestration",
        &[
            "logic-analyzer-graph-compiler",
            "logic-analyzer-graph-runtime",
        ],
    );
}
