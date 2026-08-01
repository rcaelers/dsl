#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/target/wasm-app/dist"
WASM_BINDGEN_VERSION="${WASM_BINDGEN_VERSION:-0.2.126}"

command -v wasm-bindgen >/dev/null 2>&1 || {
  echo "wasm-bindgen is required. Install with:"
  echo "  cargo install wasm-bindgen-cli --version ${WASM_BINDGEN_VERSION}"
  exit 1
}
command -v node >/dev/null 2>&1 || {
  echo "node is required to validate the Web Worker bootstrap"
  exit 1
}
command -v rg >/dev/null 2>&1 || {
  echo "rg is required to validate the generated worker entry point"
  exit 1
}

node --check "${ROOT_DIR}/crates/logic_analyzer_platform/src/platform/web_worker_bootstrap.js"

BUILD_ARGS=(
  --manifest-path "${ROOT_DIR}/Cargo.toml"
  --package logic-analyzer-app-web
  --target wasm32-unknown-unknown
  --release
)
if [[ -n "${WASM_APP_FEATURES:-}" ]]; then
  BUILD_ARGS+=(--features "${WASM_APP_FEATURES}")
fi
cargo build "${BUILD_ARGS[@]}"

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}/pkg" "${OUT_DIR}/icons"
cp "${ROOT_DIR}"/crates/app_web/web/* "${OUT_DIR}/"
cp "${ROOT_DIR}/resources/icons/LogicConduit-icon-1024.svg" \
  "${OUT_DIR}/icons/logic-conduit.svg"
cp "${ROOT_DIR}/resources/icons/LogicConduit.iconset/icon_32x32.png" \
  "${OUT_DIR}/icons/logic-conduit-32.png"
cp "${ROOT_DIR}/resources/icons/LogicConduit.iconset/icon_256x256.png" \
  "${OUT_DIR}/icons/logic-conduit-256.png"
cp "${ROOT_DIR}/resources/icons/LogicConduit.iconset/icon_512x512.png" \
  "${OUT_DIR}/icons/logic-conduit-512.png"

wasm-bindgen \
  "${ROOT_DIR}/target/wasm32-unknown-unknown/release/logic_conduit.wasm" \
  --target web \
  --out-dir "${OUT_DIR}/pkg"

node --check "${OUT_DIR}/main.js"
node --check "${OUT_DIR}/pkg/logic_conduit.js"
rg --quiet "export function executePortableWorkerOperation" \
  "${OUT_DIR}/pkg/logic_conduit.js"

echo "WASM app written to ${OUT_DIR}"
