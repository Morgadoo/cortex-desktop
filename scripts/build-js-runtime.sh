#!/usr/bin/env bash
#
# Builds the interpreter that runs the tools Cortex's assistant writes (a WebAssembly component
# with the Boa JavaScript engine) and copies it to src-tauri/crates/cortex-sandbox/wasm/js-runtime.wasm,
# which the app embeds. Run it after changing the js-runtime crate or wasm/wit/ next to it.
#
# Needs the wasm32-wasip2 target: rustup target add wasm32-wasip2

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
crate="$root/src-tauri/crates/cortex-sandbox/wasm/js-runtime"

cargo build --release --target wasm32-wasip2 --manifest-path "$crate/Cargo.toml"
cp "$crate/target/wasm32-wasip2/release/js_runtime.wasm" "$root/src-tauri/crates/cortex-sandbox/wasm/js-runtime.wasm"
sha256sum "$root/src-tauri/crates/cortex-sandbox/wasm/js-runtime.wasm"
