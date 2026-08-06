use std::collections::BTreeSet;
use std::process::Command;

const PLATFORM_DOMAIN_CRATES: &[&str] = &[
    "logic-analyzer-capture-export",
    "logic-analyzer-graph-capabilities",
    "logic-analyzer-graph-nodes",
    "logic-analyzer-graph-orchestration",
    "logic-analyzer-graph-runtime",
    "logic-analyzer-processing",
    "logic-analyzer-ui",
    "node-graph",
    "signal-capture",
    "signal-capture-session",
    "signal-derived",
];

// Temporary exceptions owned by composition.platform-ui-inversion. The test
// deliberately requires this list to equal the remaining manifest violations,
// so removing an edge also requires deleting its exception.
const TEMPORARY_PLATFORM_DOMAIN_EDGES: &[&str] = &[
    "logic-analyzer-graph-orchestration",
    "logic-analyzer-processing",
    "signal-capture",
    "signal-capture-session",
    "signal-derived",
];

fn workspace_metadata() -> serde_json::Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must be JSON")
}

fn package<'a>(metadata: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    metadata["packages"]
        .as_array()
        .expect("metadata packages must be an array")
        .iter()
        .find(|package| package["name"] == name)
        .unwrap_or_else(|| panic!("workspace must contain {name}"))
}

fn non_dev_dependencies(package: &serde_json::Value) -> BTreeSet<&str> {
    package["dependencies"]
        .as_array()
        .expect("package dependencies must be an array")
        .iter()
        .filter(|dependency| dependency["kind"] != "dev")
        .filter_map(|dependency| dependency["name"].as_str())
        .collect()
}

#[test]
fn platform_domain_edges_match_the_explicit_temporary_allowlist() {
    let metadata = workspace_metadata();
    let actual = non_dev_dependencies(package(&metadata, "logic-analyzer-platform"))
        .into_iter()
        .filter(|name| PLATFORM_DOMAIN_CRATES.contains(name))
        .collect::<BTreeSet<_>>();
    let expected = TEMPORARY_PLATFORM_DOMAIN_EDGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    assert_eq!(
        actual, expected,
        "platform domain dependencies must be removed with their TODO allowlist entries"
    );
}

#[test]
fn capture_export_owner_does_not_depend_on_ui_or_platform() {
    let metadata = workspace_metadata();
    let dependencies = non_dev_dependencies(package(&metadata, "logic-analyzer-capture-export"));

    assert!(
        dependencies.is_disjoint(&BTreeSet::from([
            "logic-analyzer-platform",
            "logic-analyzer-ui",
        ])),
        "the capture-export service contract and implementation must remain independent of UI and platform"
    );
}
