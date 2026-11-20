//! The interpreter component: compiled once, cached on disk, and shared by every run.

use super::*;

/// The engine, linker and compiled interpreter, made once per process.
pub(super) struct Runtime {
    pub(super) engine: Engine,
    pub(super) linker: Linker<HostState>,
    pub(super) component: Component,
}

pub(super) static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where the compiled interpreter is kept between runs of the app. Set once, before the first tool
/// runs; without it the interpreter is compiled once per process.
pub fn set_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

pub(super) fn engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|e| e.to_string())
}

/// The interpreter, compiled: from `cache` when a copy compiled from these exact bytes by a
/// compatible engine is there, otherwise compiled and kept there. Compiling takes seconds; loading
/// a kept copy takes milliseconds. A copy that does not load is compiled again and replaced.
pub(super) fn load_or_compile(
    engine: &Engine,
    wasm: &[u8],
    cache: Option<&Path>,
) -> Result<Component, String> {
    use sha2::{Digest, Sha256};
    use std::hash::{Hash, Hasher};
    let Some(cache) = cache else {
        return Component::new(engine, wasm).map_err(|e| e.to_string());
    };
    let mut compat = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut compat);
    let digest: String = Sha256::digest(wasm)
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect();
    let file = cache.join(format!(
        "js-runtime-{digest}-{:016x}.cwasm",
        compat.finish()
    ));
    if file.exists() {
        // SAFETY: the file is in the app's own data folder, named by the hash of the component
        // it was compiled from and of the engine that compiled it, and wasmtime checks its header
        // and compatibility before using it. Whoever can replace it can already run code as the
        // user, so it adds no reach.
        if let Ok(component) = unsafe { Component::deserialize_file(engine, &file) } {
            return Ok(component);
        }
    }
    let compiled = engine
        .precompile_component(wasm)
        .map_err(|e| e.to_string())?;
    if std::fs::create_dir_all(cache).is_ok() {
        let partial = file.with_extension("partial");
        if std::fs::write(&partial, &compiled).is_ok() {
            let _ = std::fs::rename(&partial, &file);
        }
    }
    // SAFETY: these bytes were compiled by this engine just now.
    unsafe { Component::deserialize(engine, &compiled) }.map_err(|e| e.to_string())
}

pub(super) fn runtime() -> Result<&'static Runtime, String> {
    static RUNTIME_CELL: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME_CELL
        .get_or_init(|| {
            let engine = engine()?;
            // Advance the epoch on a timer so running guests can be interrupted.
            let weak = engine.weak();
            std::thread::spawn(move || {
                while let Some(engine) = weak.upgrade() {
                    engine.increment_epoch();
                    drop(engine);
                    std::thread::sleep(TICK);
                }
            });
            let mut linker = Linker::new(&engine);
            // The interpreter's standard library imports WASI; it gets an empty context.
            wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|e| e.to_string())?;
            cortex::script::host::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
                .map_err(|e| e.to_string())?;
            let component =
                load_or_compile(&engine, RUNTIME, CACHE_DIR.get().map(PathBuf::as_path))?;
            Ok(Runtime {
                engine,
                linker,
                component,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}
