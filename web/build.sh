#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --target wasm32-unknown-unknown --lib
"${NOTIST_WASM_BINDGEN:-wasm-bindgen}" --target web --out-dir web/pkg target/wasm32-unknown-unknown/debug/notist.wasm
