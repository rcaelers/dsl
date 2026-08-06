use std::collections::BTreeSet;
use std::process::Command;

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

#[test]
fn platform_contract_crates_have_no_product_dependencies() {
    let metadata = workspace_metadata();

    for owner in ["platform-artifacts", "platform-runtime"] {
        let dependencies = local_non_dev_dependencies(package(&metadata, owner));
        assert!(
            dependencies.is_empty(),
            "{owner} must remain independent of every workspace crate: {dependencies:?}"
        );
    }
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

fn local_non_dev_dependencies(package: &serde_json::Value) -> BTreeSet<&str> {
    package["dependencies"]
        .as_array()
        .expect("package dependencies must be an array")
        .iter()
        .filter(|dependency| dependency["kind"] != "dev")
        .filter(|dependency| !dependency["path"].is_null())
        .filter_map(|dependency| dependency["name"].as_str())
        .collect()
}

#[test]
fn platform_depends_only_on_neutral_contract_owners() {
    let metadata = workspace_metadata();
    let dependencies = local_non_dev_dependencies(package(&metadata, "logic-analyzer-platform"));

    assert_eq!(
        dependencies,
        BTreeSet::from(["platform-artifacts", "platform-runtime"]),
        "platform may depend only on its neutral contract owners"
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
