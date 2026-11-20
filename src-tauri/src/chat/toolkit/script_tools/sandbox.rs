//! The WebAssembly sandbox: the `cortex-sandbox` crate.

pub use cortex_sandbox::*;

/// Keeps the compiled interpreter in `<config>/cache/wasm`, beside the app's data and outside
/// what is synced, so it is compiled once, not on every start. The first call wins.
pub(in crate::chat::toolkit) fn cache_in(config_dir: &std::path::Path) {
    set_cache_dir(config_dir.join("cache").join("wasm"));
}
