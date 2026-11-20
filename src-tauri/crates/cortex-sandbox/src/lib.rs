//! The WebAssembly sandbox that runs tools the assistant writes.
//!
//! A script tool is JavaScript run by one audited interpreter component
//! (`wasm/js-runtime.wasm` in this crate, built from `wasm/js-runtime/` by
//! `scripts/build-js-runtime.sh`) that implements the `script-tool` world
//! in `wasm/wit/script.wit`. Each call gets a fresh instance with hard limits on CPU (fuel), memory
//! and time, an empty WASI context (no files, environment, arguments or sockets), and only the host
//! functions its capabilities allow. A denial goes back to the script as an error, so the model can
//! read why.
//!
//! Adapted from the `agent-kernel` reference build, with its known gaps closed: response bodies
//! are capped while they stream, a host name is resolved and the connection pinned to a public
//! address (no DNS rebinding), and the component is compiled once per process.

use std::net::{IpAddr, SocketAddr};
use std::path::{Component as PathPart, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "wasm/wit",
    world: "script-tool",
    imports: { default: async },
    exports: { default: async },
});

use cortex::script::host::{Host, Response};

/// The interpreter component, built by `scripts/build-js-runtime.sh`.
const RUNTIME: &[u8] = include_bytes!("../wasm/js-runtime.wasm");

/// How often the epoch advances; time limits are counted in ticks.
const TICK: Duration = Duration::from_millis(10);
const MAX_READ_BYTES: u64 = 1 << 20;
const MAX_WRITE_BYTES: usize = 1 << 20;
/// What a script may return.
const MAX_RESULT_BYTES: usize = 1 << 20;
const MAX_HTTP_BODY_BYTES: usize = 1 << 20;
const MAX_LOG_LINES: usize = 200;

/// What a tool may reach. The default grants nothing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Capabilities {
    /// Exact host names the tool may call over HTTPS.
    pub http_hosts: Vec<String>,
    /// Folders of the chat's workspace the tool may read ("" for all of it).
    pub read_prefixes: Vec<String>,
    /// Folders of the chat's workspace the tool may write ("" for all of it). A tool that may
    /// write changes something, so it is never offered as read-only.
    pub write_prefixes: Vec<String>,
    /// Other approved tools it may call, by name.
    pub tools: Vec<String>,
    /// Credentials for its requests, as `host=SECRET`: the host attaches the vault secret named
    /// SECRET to requests to that host. The script never sees the value.
    pub credentials: Vec<String>,
}

impl Capabilities {
    /// Both sets together.
    pub fn with(&self, more: &Capabilities) -> Capabilities {
        let join = |a: &[String], b: &[String]| {
            let mut out = a.to_vec();
            out.extend(b.iter().filter(|x| !a.contains(x)).cloned());
            out
        };
        Capabilities {
            http_hosts: join(&self.http_hosts, &more.http_hosts),
            read_prefixes: join(&self.read_prefixes, &more.read_prefixes),
            write_prefixes: join(&self.write_prefixes, &more.write_prefixes),
            tools: join(&self.tools, &more.tools),
            credentials: join(&self.credentials, &more.credentials),
        }
    }

    /// Only what is also in `limit`.
    pub fn within(&self, limit: &Capabilities) -> Capabilities {
        let keep =
            |a: &[String], b: &[String]| a.iter().filter(|x| b.contains(x)).cloned().collect();
        Capabilities {
            http_hosts: keep(&self.http_hosts, &limit.http_hosts),
            read_prefixes: keep(&self.read_prefixes, &limit.read_prefixes),
            write_prefixes: keep(&self.write_prefixes, &limit.write_prefixes),
            tools: keep(&self.tools, &limit.tools),
            credentials: keep(&self.credentials, &limit.credentials),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.http_hosts.is_empty()
            && self.read_prefixes.is_empty()
            && self.write_prefixes.is_empty()
            && self.tools.is_empty()
            && self.credentials.is_empty()
    }
}

/// Runs another tool for a script that may call it: the host's own lookup, limits and gates.
pub type ToolCaller = std::sync::Arc<
    dyn Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Resource ceilings for one call.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Wasm instructions, roughly. Bounds CPU.
    pub fuel: u64,
    /// Linear memory, in bytes.
    pub memory_bytes: usize,
    /// Wall-clock time, including time spent waiting on host calls.
    pub timeout: Duration,
    /// A fixed clock and a seeded random generator, so the same arguments give the same result:
    /// what a regression check needs from a tool that reads the date or draws random numbers.
    pub deterministic: bool,
}

/// The time a deterministic run sees: 2026-01-01T00:00:00Z, standing still.
struct FixedWallClock;

impl wasmtime_wasi::HostWallClock for FixedWallClock {
    fn resolution(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn now(&self) -> Duration {
        Duration::from_secs(1_767_225_600)
    }
}

struct FixedMonotonicClock;

impl wasmtime_wasi::HostMonotonicClock for FixedMonotonicClock {
    fn resolution(&self) -> u64 {
        1
    }
    fn now(&self) -> u64 {
        0
    }
}

/// The guest's WASI context: empty, inheriting nothing; for a deterministic run, also a fixed
/// clock and seeded randomness.
fn wasi_context(deterministic: bool) -> WasiCtx {
    use rand::SeedableRng;
    let mut builder = WasiCtxBuilder::new();
    if deterministic {
        builder
            .wall_clock(FixedWallClock)
            .monotonic_clock(FixedMonotonicClock)
            .secure_random(rand::rngs::StdRng::seed_from_u64(7))
            .insecure_random(rand::rngs::StdRng::seed_from_u64(7))
            .insecure_random_seed(7);
    }
    builder.build()
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 5_000_000_000,
            memory_bytes: 128 << 20,
            timeout: Duration::from_secs(20),
            deterministic: false,
        }
    }
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SandboxError {
    /// The script ran and failed, or a capability was denied.
    #[error("{0}")]
    Script(String),
    #[error("the tool used up its CPU budget")]
    OutOfFuel,
    #[error("the tool went over its memory limit")]
    OutOfMemory,
    #[error("the tool went over its time limit")]
    Timeout,
    #[error("the tool crashed: {0}")]
    Crashed(String),
}

/// The folder `read` and `write` can reach, and how its files' bytes are encoded on disk.
#[derive(Clone)]
pub struct Workspace {
    pub root: PathBuf,
    /// Turns a file's bytes, as they are on disk, into what the script reads (the app opens what
    /// it sealed). The sandbox does the reading itself, safely.
    pub read: fn(&Path, Vec<u8>) -> std::io::Result<Vec<u8>>,
    /// Turns what the script writes into the bytes to put on disk (the app seals them).
    pub write: fn(&Path, &[u8]) -> std::io::Result<Vec<u8>>,
}

/// Per-call state, dropped with the instance when the call ends.
struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    capabilities: Capabilities,
    workspace: Option<Workspace>,
    tools: Option<ToolCaller>,
    /// Secret values by name, for the credentials the tool was granted.
    secrets: std::collections::HashMap<String, String>,
    memory_limit: usize,
    memory_exceeded: bool,
    logs: Vec<String>,
}

/// Runs `script`'s `main` with JSON `args`, in a fresh instance, within `limits` and
/// `capabilities`. `workspace` is the only folder `read` can reach. Log lines come back apart
/// from the result: they are for the activity log, never the model.
pub async fn run(
    script: &str,
    args: &str,
    capabilities: &Capabilities,
    limits: Limits,
    workspace: Option<Workspace>,
) -> (Result<String, SandboxError>, Vec<String>) {
    run_with_tools(script, args, capabilities, limits, workspace, None).await
}

/// [`run`], with a way to call the other tools `capabilities.tools` names.
pub async fn run_with_tools(
    script: &str,
    args: &str,
    capabilities: &Capabilities,
    limits: Limits,
    workspace: Option<Workspace>,
    tools: Option<ToolCaller>,
) -> (Result<String, SandboxError>, Vec<String>) {
    run_with_secrets(
        script,
        args,
        capabilities,
        limits,
        workspace,
        tools,
        Default::default(),
    )
    .await
}

/// [`run_with_tools`], with the values of the vault secrets its `credentials` name.
pub async fn run_with_secrets(
    script: &str,
    args: &str,
    capabilities: &Capabilities,
    limits: Limits,
    workspace: Option<Workspace>,
    tools: Option<ToolCaller>,
    secrets: std::collections::HashMap<String, String>,
) -> (Result<String, SandboxError>, Vec<String>) {
    let runtime = match runtime() {
        Ok(r) => r,
        Err(e) => return (Err(SandboxError::Crashed(e)), Vec::new()),
    };
    let state = HostState {
        // Nothing inherited: no stdio, arguments, environment, folders or network.
        wasi: wasi_context(limits.deterministic),
        table: ResourceTable::new(),
        capabilities: capabilities.clone(),
        workspace,
        tools,
        secrets,
        memory_limit: limits.memory_bytes,
        memory_exceeded: false,
        logs: Vec::new(),
    };
    let mut store = Store::new(&runtime.engine, state);
    store.limiter(|state| state);
    if let Err(e) = store.set_fuel(limits.fuel) {
        return (Err(SandboxError::Crashed(e.to_string())), Vec::new());
    }
    // A computing guest yields its thread every tick instead of holding it, so a CPU-bound tool
    // never starves the runtime's other work; the timeout below then stops it.
    store.set_epoch_deadline(1);
    store.epoch_deadline_async_yield_and_update(1);

    let call = async {
        let instance =
            ScriptTool::instantiate_async(&mut store, &runtime.component, &runtime.linker).await?;
        instance.call_run(&mut store, script, args).await
    };
    // The timeout stops a guest whether it computes (it yields every tick) or waits on the host.
    let outcome = tokio::time::timeout(limits.timeout, call).await;
    let logs = std::mem::take(&mut store.data_mut().logs);
    let result = match outcome {
        Err(_elapsed) => Err(SandboxError::Timeout),
        Ok(Ok(Ok(output))) if output.len() > MAX_RESULT_BYTES => Err(SandboxError::Script(
            format!("the result is larger than {MAX_RESULT_BYTES} bytes; return less, or write it to a file"),
        )),
        Ok(Ok(Ok(output))) => Ok(output),
        Ok(Ok(Err(message))) => Err(SandboxError::Script(message)),
        Ok(Err(trap)) => Err(if store.data().memory_exceeded {
            SandboxError::OutOfMemory
        } else {
            match trap.downcast_ref::<Trap>() {
                Some(Trap::OutOfFuel) => SandboxError::OutOfFuel,
                Some(Trap::Interrupt) => SandboxError::Timeout,
                _ => SandboxError::Crashed(trap.to_string()),
            }
        }),
    };
    (result, logs)
}

mod host;
mod reach;
mod runtime;
use host::*;
use reach::*;
pub use runtime::*;

#[cfg(test)]
mod tests;
