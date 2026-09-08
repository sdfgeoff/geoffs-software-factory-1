use std::{collections::BTreeSet, process::Command};

use serde_json::Value;

const FORBIDDEN: [&str; 3] = ["multi-agent-board", "rusqlite", "sqlx"];

#[test]
fn isolated_crates_have_only_allowed_dependencies() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata should run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_dependencies(
        &metadata,
        "multi-agent-board-protocol",
        &["serde", "serde_json"],
    );
    assert_dependencies(
        &metadata,
        "multi-agent-board-mcp",
        &[
            "multi-agent-board-protocol",
            "reqwest",
            "schemars",
            "serde",
            "serde_json",
            "tokio",
        ],
    );
}

fn assert_dependencies(metadata: &Value, package_name: &str, allowed: &[&str]) {
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == package_name)
        .unwrap_or_else(|| panic!("missing package {package_name}"));
    let actual = package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|dependency| dependency["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();

    for forbidden in FORBIDDEN {
        assert!(
            !actual.contains(forbidden),
            "{package_name} must not depend on {forbidden}"
        );
    }
    assert_eq!(
        actual, allowed,
        "update the reviewed allowlist intentionally when changing {package_name} dependencies"
    );
}
