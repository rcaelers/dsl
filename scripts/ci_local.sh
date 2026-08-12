#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_TARGET="wasm32-unknown-unknown"
WASM_RUNNER="wasm-bindgen-test-runner"

cd "${ROOT_DIR}"

usage() {
  cat <<'EOF'
Usage: scripts/ci_local.sh [job]

Run the commands from .github/workflows/ci.yml on the local checkout.
Jobs run serially; "all" is the default.

Jobs:
  all
  architecture
  test-crates
  test-integration
  clippy
  check-developer-tools
  check-wasm
EOF
}

section() {
  echo
  echo "==> $1"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    return 1
  fi
}

check_common_prerequisites() {
  require_command cargo
  require_command ruby

  if [[ "$(uname -s)" == "Linux" ]]; then
    require_command pkg-config
    if ! pkg-config --exists wayland-client; then
      echo "error: wayland-client development files are unavailable" >&2
      echo "Install the CI dependency with:" >&2
      echo "  sudo apt-get install --no-install-recommends libwayland-dev" >&2
      return 1
    fi
  fi
}

check_wasm_prerequisites() {
  require_command rustup
  if ! rustup target list --installed | while IFS= read -r target; do
    [[ "${target}" == "${WASM_TARGET}" ]] && exit 0
  done; then
    echo "error: Rust target ${WASM_TARGET} is not installed" >&2
    echo "Install it with: rustup target add ${WASM_TARGET}" >&2
    return 1
  fi
  if ! command -v "${WASM_RUNNER}" >/dev/null 2>&1; then
    echo "error: ${WASM_RUNNER} is not installed" >&2
    echo "Install the pinned CI version with:" >&2
    echo "  cargo install wasm-bindgen-cli --version 0.2.126 --locked" >&2
    return 1
  fi
  require_command wasm-bindgen
  require_command node
}

run_architecture() {
  section "architecture: Rust module structure"
  ruby scripts/check_rust_module_structure.rb

  section "architecture: public function documentation"
  ruby scripts/check_public_function_docs.rb

  section "architecture: native/web platform boundaries"
  ruby scripts/check_platform_boundaries_test.rb
  ruby scripts/check_platform_boundaries.rb

  section "architecture: visibility"
  RUSTFLAGS="-D unreachable-pub" cargo check --workspace --all-targets

  section "architecture: public API documentation"
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --lib
}

run_test_crates() {
  section "test-crates: discover workspace crates"
  local packages
  packages="$({
    cargo metadata --no-deps --format-version 1
  } | ruby -rjson -e '
    metadata = JSON.parse(STDIN.read)
    root_manifest = File.join(metadata.fetch("workspace_root"), "Cargo.toml")
    metadata.fetch("packages").each do |package|
      puts package.fetch("name") unless package.fetch("manifest_path") == root_manifest
    end
  ')"

  local package
  for package in ${packages}; do
    section "test-crates: ${package}"
    cargo test -p "${package}"
  done
}

run_test_integration() {
  section "test-integration: top-level integration package"
  cargo test -p logic-analyzer-examples

  section "test-integration: native compile-time plugin linking"
  cargo test -p logic-analyzer-app-native --features example-plugin
}

run_clippy() {
  section "clippy: all targets and features"
  cargo clippy --workspace --all-targets --all-features -- -D warnings
}

run_developer_tools() {
  section "check-developer-tools: deterministic benchmark commands"
  cargo check -p signal-derived --bin derived-word-store-bench
  cargo check -p logic-analyzer-examples --features developer-tools \
    --bin u3pro16-streaming-bench
  cargo check -p logic-analyzer-examples --bench compiler_capture

  section "check-developer-tools: manual validation commands"
  cargo check -p logic-analyzer-app-native --features developer-tools \
    --bin u3pro16-hardware-validation
  cargo check -p logic-analyzer-app-native --features developer-tools \
    --bin sigrok-upstream-validation
}

wasm_test() {
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="${WASM_RUNNER}" cargo test "$@"
}

run_wasm() {
  check_wasm_prerequisites

  section "check-wasm: portable web composition"
  RUSTFLAGS="-D unreachable-pub" cargo check -p logic-analyzer-app-web \
    --target "${WASM_TARGET}" --all-targets --all-features

  section "check-wasm: portable capture-node parity"
  wasm_test -p logic-analyzer-graph-nodes --target "${WASM_TARGET}" \
    --lib platform_parity_tests

  section "check-wasm: portable storage"
  wasm_test -p platform-artifacts --target "${WASM_TARGET}" --lib wasm_store_tests
  wasm_test -p signal-derived --target "${WASM_TARGET}" --lib wasm_store_tests

  section "check-wasm: browser artifact persistence in OPFS"
  wasm_test -p platform --target "${WASM_TARGET}" --lib \
    published_artifact_rehydrates_from_a_second_opfs_worker

  section "check-wasm: browser capture-file import adapters"
  wasm_test -p logic-analyzer-app-web --target "${WASM_TARGET}" \
    --lib web_file_import_tests

  section "check-wasm: browser graph-document adapters"
  wasm_test -p platform --target "${WASM_TARGET}" --lib web_document_tests

  section "check-wasm: portable worker scheduling"
  wasm_test -p platform-runtime --target "${WASM_TARGET}" \
    --lib worker_operation_queue_tests
  wasm_test -p platform-runtime --target "${WASM_TARGET}" \
    --lib worker_messages_round_trip_as_owned_data

  section "check-wasm: UI with compile-time plugin constructors"
  RUSTFLAGS="-D unreachable-pub" WASM_APP_FEATURES="example-plugin" \
    scripts/build_wasm_app.sh
}

run_job() {
  case "$1" in
    architecture) run_architecture ;;
    test-crates) run_test_crates ;;
    test-integration) run_test_integration ;;
    clippy) run_clippy ;;
    check-developer-tools) run_developer_tools ;;
    check-wasm) run_wasm ;;
    *)
      echo "error: unknown CI job: $1" >&2
      usage >&2
      return 2
      ;;
  esac
}

main() {
  if [[ $# -gt 1 ]] || [[ "${1:-}" == "--help" ]] || [[ "${1:-}" == "-h" ]]; then
    usage
    [[ $# -le 1 ]] || return 2
    return 0
  fi

  check_common_prerequisites

  local job="${1:-all}"
  if [[ "${job}" == "all" ]]; then
    run_architecture
    run_test_crates
    run_test_integration
    run_clippy
    run_developer_tools
    run_wasm
  else
    run_job "${job}"
  fi

  section "local CI completed successfully"
}

main "$@"
