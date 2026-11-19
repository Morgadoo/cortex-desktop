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
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 5_000_000_000,
            memory_bytes: 128 << 20,
            timeout: Duration::from_secs(20),
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

/// The folder `read` and `write` can reach, and how its files are read and written (the app
/// seals what it writes and opens what it sealed).
#[derive(Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub read: fn(&Path) -> std::io::Result<Vec<u8>>,
    pub write: fn(&Path, &[u8]) -> std::io::Result<()>,
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

/// The secret to send to `host`: one the tool was granted for that host, whose value is known.
fn credential_for<'a>(
    capabilities: &Capabilities,
    secrets: &'a std::collections::HashMap<String, String>,
    host: &str,
) -> Option<&'a str> {
    capabilities.credentials.iter().find_map(|c| {
        let (for_host, name) = c.split_once('=')?;
        for_host
            .eq_ignore_ascii_case(host)
            .then(|| secrets.get(name).map(String::as_str))
            .flatten()
    })
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ResourceLimiter for HostState {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_limit {
            self.memory_exceeded = true;
            wasmtime::bail!("memory limit exceeded");
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 100_000)
    }
}

/// Addresses a tool may never reach, whatever name points at them.
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0 // "this network"
                || a == 100 && (b & 0xc0) == 64 // carrier-grade NAT
                || a == 192 && b == 0 && v4.octets()[2] == 0 // IETF protocol assignments
                || a == 198 && (b & 0xfe) == 18 // benchmarking
                || a >= 240 // reserved
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            // An IPv4 address carried inside an IPv6 one is judged as that IPv4 address.
            let embedded = |v4: [u8; 4]| is_private(IpAddr::V4(v4.into()));
            let low = [(s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] & 0xffc0) == 0xfec0 // site local (deprecated)
                || v6.to_ipv4_mapped().is_some_and(|v4| is_private(IpAddr::V4(v4)))
                // IPv4-compatible ::a.b.c.d
                || (s[..6] == [0; 6] && embedded(low))
                // NAT64 64:ff9b::/96
                || (s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0; 4] && embedded(low))
                // 6to4 2002:aabb:ccdd::/48
                || (s[0] == 0x2002
                    && embedded([
                        (s[1] >> 8) as u8,
                        s[1] as u8,
                        (s[2] >> 8) as u8,
                        s[2] as u8,
                    ]))
                // Teredo 2001::/32 hides the client's address; refuse it whole.
                || (s[0] == 0x2001 && s[1] == 0)
        }
    }
}

/// Every host function checks a capability before it does anything, and returns an error to the
/// script instead of trapping, so a well-behaved tool can report the denial.
impl Host for HostState {
    async fn log(&mut self, msg: String) {
        if self.logs.len() < MAX_LOG_LINES {
            self.logs.push(msg.chars().take(500).collect());
        }
    }

    async fn workspace_read(&mut self, path: String) -> Result<Vec<u8>, String> {
        let Some(Workspace {
            root: workspace,
            read,
            ..
        }) = self.workspace.clone()
        else {
            return Err("denied: this call has no workspace".into());
        };
        let relative = Path::new(&path);
        if !relative
            .components()
            .all(|part| matches!(part, PathPart::Normal(_)))
        {
            return Err(format!("denied: `{path}` is not a plain relative path"));
        }
        if !self
            .capabilities
            .read_prefixes
            .iter()
            .any(|prefix| prefix.is_empty() || relative.starts_with(prefix))
        {
            return Err(format!("denied: no read capability for `{path}`"));
        }
        // Resolve symlinks, then confirm the target is still inside the workspace.
        let root = tokio::fs::canonicalize(&workspace)
            .await
            .map_err(|_| "the workspace is empty".to_string())?;
        let target = tokio::fs::canonicalize(root.join(relative))
            .await
            .map_err(|_| format!("not found: `{path}`"))?;
        if !target.starts_with(&root) {
            return Err(format!("denied: `{path}` leaves the workspace"));
        }
        let size = tokio::fs::metadata(&target)
            .await
            .map_err(|_| format!("not found: `{path}`"))?
            .len();
        if size > MAX_READ_BYTES {
            return Err(format!(
                "denied: `{path}` is larger than {MAX_READ_BYTES} bytes"
            ));
        }
        read(&target).map_err(|_| format!("could not read `{path}`"))
    }

    async fn workspace_write(&mut self, path: String, data: Vec<u8>) -> Result<(), String> {
        let Some(Workspace {
            root: workspace,
            write,
            ..
        }) = self.workspace.clone()
        else {
            return Err("denied: this call has no workspace".into());
        };
        let relative = Path::new(&path);
        if path.is_empty()
            || !relative
                .components()
                .all(|part| matches!(part, PathPart::Normal(_)))
        {
            return Err(format!("denied: `{path}` is not a plain relative path"));
        }
        if !self
            .capabilities
            .write_prefixes
            .iter()
            .any(|prefix| prefix.is_empty() || relative.starts_with(prefix))
        {
            return Err(format!("denied: no write capability for `{path}`"));
        }
        if data.len() > MAX_WRITE_BYTES {
            return Err(format!(
                "denied: writes are at most {MAX_WRITE_BYTES} bytes"
            ));
        }
        tokio::fs::create_dir_all(&workspace)
            .await
            .map_err(|e| e.to_string())?;
        let root = tokio::fs::canonicalize(&workspace)
            .await
            .map_err(|e| e.to_string())?;
        // Folders are made one at a time inside the workspace: a link anywhere on the way is
        // refused before anything is made past it, so nothing is created outside.
        let mut parent = root.clone();
        let folders: Vec<_> = relative.components().collect();
        let (file, folders) = folders.split_last().ok_or("denied: no file name")?;
        for folder in folders {
            parent.push(folder);
            match tokio::fs::symlink_metadata(&parent).await {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(format!(
                        "denied: `{path}` leaves the workspace through a link"
                    ))
                }
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Err(format!("denied: `{path}` goes through a file")),
                Err(_) => tokio::fs::create_dir(&parent)
                    .await
                    .map_err(|_| format!("could not make the folder for `{path}`"))?,
            }
        }
        let target = parent.join(file);
        if tokio::fs::symlink_metadata(&target)
            .await
            .is_ok_and(|m| m.file_type().is_symlink() || m.is_dir())
        {
            return Err(format!("denied: `{path}` is a link or a folder"));
        }
        write(&target, &data).map_err(|_| format!("could not write `{path}`"))
    }

    async fn call_tool(&mut self, name: String, args: String) -> Result<String, String> {
        if !self.capabilities.tools.contains(&name) {
            return Err(format!("denied: no capability to call `{name}`"));
        }
        let Some(caller) = self.tools.clone() else {
            return Err("denied: tools cannot be called here".into());
        };
        caller(name, args).await
    }

    async fn http_request(
        &mut self,
        method: String,
        url: String,
        body: Option<Vec<u8>>,
    ) -> Result<Response, String> {
        let parsed = url::Url::parse(&url).map_err(|_| "denied: invalid URL".to_string())?;
        if parsed.scheme() != "https" {
            return Err("denied: only https is allowed".into());
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err("denied: credentials in the URL are not allowed".into());
        }
        // `Domain` excludes IP literals, so 169.254.169.254 and friends never match.
        let Some(url::Host::Domain(host)) = parsed.host() else {
            return Err("denied: the URL must name a host, not an IP address".into());
        };
        let host = host.to_string();
        if !self
            .capabilities
            .http_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&host))
        {
            return Err(format!("denied: no http capability for `{host}`"));
        }
        let method = match method.to_ascii_uppercase().as_str() {
            "" | "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            other => return Err(format!("denied: method `{other}` is not allowed")),
        };
        // Resolve once, refuse private addresses, and connect to the address that was checked,
        // so a name cannot point at the local network between the check and the request.
        let port = parsed.port_or_known_default().unwrap_or(443);
        let resolved = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| format!("could not resolve `{host}`"))?;
        let address = checked_address(resolved)
            .ok_or_else(|| format!("denied: `{host}` points at a private address"))?;
        let client = pinned_client(&host, address)?;
        let mut request = client.request(method, parsed);
        if let Some(body) = body {
            request = request.body(body);
        }
        if let Some(secret) = credential_for(&self.capabilities, &self.secrets, &host) {
            request = request.bearer_auth(secret);
        }
        let mut response = request
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e.without_url()))?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("request failed: {}", e.without_url()))?
        {
            if body.len() + chunk.len() > MAX_HTTP_BODY_BYTES {
                return Err(format!(
                    "denied: the response is larger than {MAX_HTTP_BODY_BYTES} bytes"
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response { status, body })
    }
}

/// The first public address a name resolved to; none when every one is private.
fn checked_address(resolved: impl IntoIterator<Item = SocketAddr>) -> Option<SocketAddr> {
    resolved.into_iter().find(|a| !is_private(a.ip()))
}

/// A client that reaches `host` only at `address`, the one that was checked: the name is not
/// looked up again, so it cannot be pointed elsewhere between the check and the request.
fn pinned_client(host: &str, address: SocketAddr) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        // A proxy would look the name up again, past the check and the pin.
        .no_proxy()
        .resolve(host, address)
        .build()
        .map_err(|e| e.to_string())
}

/// The engine, linker and compiled interpreter, made once per process.
struct Runtime {
    engine: Engine,
    linker: Linker<HostState>,
    component: Component,
}

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where the compiled interpreter is kept between runs of the app. Set once, before the first tool
/// runs; without it the interpreter is compiled once per process.
pub fn set_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

fn engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|e| e.to_string())
}

/// The interpreter, compiled: from `cache` when a copy compiled from these exact bytes by a
/// compatible engine is there, otherwise compiled and kept there. Compiling takes seconds; loading
/// a kept copy takes milliseconds. A copy that does not load is compiled again and replaced.
fn load_or_compile(
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

fn runtime() -> Result<&'static Runtime, String> {
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
        wasi: WasiCtxBuilder::new().build(),
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn js(
        script: &str,
        args: &str,
        caps: Capabilities,
        limits: Limits,
    ) -> Result<String, SandboxError> {
        run(script, args, &caps, limits, None).await.0
    }

    #[tokio::test]
    async fn a_script_computes_and_returns_json() {
        let out = js(
            "function main(a) { return { sum: a.xs.reduce((s, x) => s + x, 0), n: a.xs.length }; }",
            r#"{"xs":[1,2,3,4]}"#,
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap();
        assert_eq!(out, r#"{"sum":10,"n":4}"#);
    }

    #[tokio::test]
    async fn arguments_never_become_code() {
        let out = js(
            "function main(a) { return a.text; }",
            r#"{"text":"\")); throw new Error(\"x"}"#,
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap();
        assert!(out.contains("throw new Error"), "{out}");
    }

    #[tokio::test]
    async fn a_script_error_comes_back_as_text() {
        let err = js(
            "function main() { throw new Error('bad input'); }",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, SandboxError::Script(m) if m.contains("bad input")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_endless_loop_is_stopped() {
        let err = js(
            "function main() { while (true) {} }",
            "{}",
            Capabilities::default(),
            Limits {
                fuel: 200_000_000,
                ..Limits::default()
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, SandboxError::OutOfFuel | SandboxError::Timeout),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_memory_hog_is_stopped() {
        let err = js(
            "function main() { const a = []; while (true) { a.push('x'.repeat(1 << 20) + a.length); } }",
            "{}",
            Capabilities::default(),
            Limits {
                fuel: u64::MAX,
                memory_bytes: 16 << 20,
                ..Limits::default()
            },
        )
        .await
        .unwrap_err();
        // The limiter fired: not a script error or a crash that only happens to follow.
        assert_eq!(err, SandboxError::OutOfMemory);
    }

    /// The host-call timeout: a tool waiting on a host function that never answers is stopped.
    #[tokio::test]
    async fn a_host_call_that_never_returns_is_stopped() {
        let hangs: ToolCaller = std::sync::Arc::new(|_name: String, _args: String| {
            Box::pin(async move {
                std::future::pending::<()>().await;
                Ok(String::new())
            })
        });
        // Compile the runtime first, so the clock below times only the call.
        let _ = js(
            "function main() { return 1; }",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await;
        let started = std::time::Instant::now();
        let (out, _) = run_with_tools(
            "function main() { return tool('slow', {}); }",
            "{}",
            &Capabilities {
                tools: vec!["slow".into()],
                ..Default::default()
            },
            Limits {
                timeout: Duration::from_millis(500),
                ..Limits::default()
            },
            None,
            Some(hangs),
        )
        .await;
        assert_eq!(out.unwrap_err(), SandboxError::Timeout);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Each call gets a fresh instance: nothing a call leaves behind is there for the next.
    #[tokio::test]
    async fn nothing_carries_over_between_calls() {
        let script = "function main() { globalThis.count = (globalThis.count || 0) + 1; return globalThis.count; }";
        for _ in 0..2 {
            let out = js(script, "{}", Capabilities::default(), Limits::default()).await;
            assert_eq!(out.unwrap(), "1");
        }
    }

    /// WASI gives the guest nothing: no environment, arguments, clock-free files or processes.
    #[tokio::test]
    async fn wasi_gives_nothing() {
        let out = js(
            "function main() { return [typeof process, typeof require, typeof Deno, typeof std, \
             typeof os, typeof fs, typeof XMLHttpRequest, typeof WebSocket].join(','); }",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            "\"undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined\""
        );
    }

    #[tokio::test]
    async fn nothing_is_reachable_without_a_capability() {
        let fetch = js(
            "function main() { return fetch('GET', 'https://example.com/', ''); }",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&fetch, SandboxError::Script(m) if m.contains("no http capability")),
            "{fetch:?}"
        );
        let read = js(
            "function main() { return read('notes.txt'); }",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&read, SandboxError::Script(m) if m.contains("denied")),
            "{read:?}"
        );
    }

    #[tokio::test]
    async fn listed_hosts_still_cannot_reach_private_addresses_or_plain_http() {
        let caps = Capabilities {
            http_hosts: vec!["localhost".into(), "127.0.0.1".into()],
            ..Default::default()
        };
        for url in [
            "https://localhost/",
            "https://127.0.0.1/",
            "http://localhost/",
        ] {
            let err = js(
                &format!("function main() {{ return fetch('GET', '{url}', ''); }}"),
                "{}",
                caps.clone(),
                Limits::default(),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(&err, SandboxError::Script(m) if m.contains("denied")),
                "{url}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn reads_stay_inside_the_granted_folders_of_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(dir.path().join("data/a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "no").unwrap();
        let caps = Capabilities {
            read_prefixes: vec!["data".into()],
            ..Default::default()
        };
        let read = |path: &str| {
            let caps = caps.clone();
            let script = format!("function main() {{ return read('{path}'); }}");
            let workspace = Workspace {
                root: dir.path().to_path_buf(),
                read: |p| std::fs::read(p),
                write: |p, b| std::fs::write(p, b),
            };
            async move {
                run(&script, "{}", &caps, Limits::default(), Some(workspace))
                    .await
                    .0
            }
        };
        assert_eq!(read("data/a.txt").await.unwrap(), "\"hello\"");
        assert!(read("secret.txt").await.is_err());
        assert!(read("../secret.txt").await.is_err());
        assert!(read("data/../secret.txt").await.is_err());
    }

    /// Write capabilities: only into the granted folders, never out of the workspace or through a
    /// link, and only up to the size cap.
    #[tokio::test]
    async fn writes_stay_inside_the_granted_folders_of_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("out")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out/escape")).unwrap();
        let caps = Capabilities {
            write_prefixes: vec!["out".into()],
            ..Default::default()
        };
        let write = |path: &str, text: &str| {
            let (caps, script) = (
                caps.clone(),
                format!("function main() {{ write('{path}', '{text}'); return 'ok'; }}"),
            );
            let workspace = Workspace {
                root: dir.path().to_path_buf(),
                read: |p| std::fs::read(p),
                write: |p, b| std::fs::write(p, b),
            };
            async move {
                run(&script, "{}", &caps, Limits::default(), Some(workspace))
                    .await
                    .0
            }
        };
        assert_eq!(write("out/report/a.txt", "hi").await.unwrap(), "\"ok\"");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out/report/a.txt")).unwrap(),
            "hi"
        );
        for (path, why) in [
            ("notes.txt", "no write capability"),
            ("../x.txt", "not a plain relative path"),
            ("/etc/x", "not a plain relative path"),
        ] {
            let err = write(path, "x").await.unwrap_err().to_string();
            assert!(err.contains(why), "{path}: {err}");
        }
        #[cfg(unix)]
        {
            let err = write("out/escape/x.txt", "x")
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("leaves the workspace"), "{err}");
            assert!(!outside.path().join("x.txt").exists());
            // A link in the folders on the way: refused before any folder is made past it.
            let err = write("out/escape/new/deeper/x.txt", "x")
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("leaves the workspace"), "{err}");
            assert!(!outside.path().join("new").exists(), "nothing made outside");
        }
        // Without the capability nothing is written, even with a workspace.
        let none = run(
            "function main() { write('out/b.txt', 'x'); }",
            "{}",
            &Capabilities::default(),
            Limits::default(),
            Some(Workspace {
                root: dir.path().to_path_buf(),
                read: |p| std::fs::read(p),
                write: |p, b| std::fs::write(p, b),
            }),
        )
        .await
        .0;
        assert!(none.is_err() && !dir.path().join("out/b.txt").exists());
    }

    /// The compilation cost: the interpreter is compiled once and kept; later loads read it back,
    /// and a damaged copy is compiled again.
    #[test]
    fn the_compiled_interpreter_is_kept_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine().unwrap();
        let kept = || {
            std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "cwasm"))
                .collect::<Vec<_>>()
        };
        load_or_compile(&engine, RUNTIME, Some(dir.path())).unwrap();
        let files = kept();
        assert_eq!(files.len(), 1);
        let started = std::time::Instant::now();
        load_or_compile(&engine, RUNTIME, Some(dir.path())).unwrap();
        let loading = started.elapsed();
        assert!(loading < Duration::from_secs(2), "{loading:?}");
        std::fs::write(&files[0], b"not a component").unwrap();
        load_or_compile(&engine, RUNTIME, Some(dir.path())).unwrap();
        assert!(std::fs::read(&files[0]).unwrap().len() > 1000, "replaced");
    }

    #[tokio::test]
    async fn logs_come_back_apart_from_the_result() {
        let (out, logs) = run(
            "function main() { log('step one'); return 1; }",
            "{}",
            &Capabilities::default(),
            Limits::default(),
            None,
        )
        .await;
        assert_eq!(out.unwrap(), "1");
        assert_eq!(logs, vec!["step one".to_string()]);
    }

    #[tokio::test]
    async fn a_program_without_main_returns_what_it_printed() {
        let out = js(
            "const args = { n: 3 };\nconsole.log('n is', args.n);\nprint({ ok: true });",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await
        .unwrap();
        assert_eq!(out, "n is 3\n{\"ok\":true}");
    }

    /// Composition: a script calls another approved tool, only one it has the capability for.
    #[tokio::test]
    async fn a_script_calls_only_the_tools_it_may() {
        let caller: ToolCaller = std::sync::Arc::new(|name: String, args: String| {
            Box::pin(async move { Ok(format!("{{\"called\":\"{name}\",\"with\":{args}}}")) })
        });
        let script = "function main() { const r = tool('double', { x: 2 }); return r.called + ':' + r.with.x; }";
        let allowed = Capabilities {
            tools: vec!["double".into()],
            ..Default::default()
        };
        let (out, _) = run_with_tools(
            script,
            "{}",
            &allowed,
            Limits::default(),
            None,
            Some(caller.clone()),
        )
        .await;
        assert_eq!(out.unwrap(), "\"double:2\"");
        let (denied, _) = run_with_tools(
            script,
            "{}",
            &Capabilities::default(),
            Limits::default(),
            None,
            Some(caller),
        )
        .await;
        assert!(
            matches!(&denied, Err(SandboxError::Script(m)) if m.contains("no capability to call")),
            "{denied:?}"
        );
    }

    /// A spinning tool yields: other work on the same runtime thread still runs meanwhile.
    #[tokio::test(flavor = "current_thread")]
    async fn a_busy_tool_does_not_starve_the_runtime() {
        // Compile first, so the clock below times only the run.
        let _ = js(
            "function main() { return 1; }",
            "{}",
            Capabilities::default(),
            Limits::default(),
        )
        .await;
        let started = std::time::Instant::now();
        let spinning = async {
            let result = js(
                "function main() { while (true) {} }",
                "{}",
                Capabilities::default(),
                Limits {
                    fuel: u64::MAX,
                    timeout: Duration::from_millis(1500),
                    ..Limits::default()
                },
            )
            .await;
            (result, started.elapsed())
        };
        let ticker = async {
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            started.elapsed()
        };
        let ((result, spun), ticked) = tokio::join!(spinning, ticker);
        assert_eq!(result.unwrap_err(), SandboxError::Timeout);
        // The guest kept running for its whole time limit, and on this one thread the ticker
        // still finished long before it: the guest yielded instead of holding the thread.
        assert!(spun >= Duration::from_millis(1400), "{spun:?}");
        assert!(ticked < Duration::from_millis(1000), "{ticked:?}");
    }

    /// DNS rebinding: the checked address is the one connected to; the name is not looked up again.
    #[tokio::test]
    async fn the_connection_goes_to_the_checked_address() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let public: SocketAddr = "93.184.216.34:443".parse().unwrap();
        let private: SocketAddr = "10.0.0.5:443".parse().unwrap();
        assert_eq!(checked_address([private, public]), Some(public));
        assert_eq!(checked_address([private]), None);
        // A name that resolves nowhere still reaches the pinned address, so DNS is not asked.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\npinned",
                )
                .await
                .unwrap();
        });
        // A proxy in the environment is not used: it would look the name up again.
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:9");
        std::env::set_var("http_proxy", "http://127.0.0.1:9");
        let client = pinned_client("rebind.invalid", address).unwrap();
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("http_proxy");
        let body = client
            .get(format!("http://rebind.invalid:{}/", address.port()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "pinned");
        server.await.unwrap();
    }

    /// The CPU budget: fuel runs out before the clock does.
    #[tokio::test]
    async fn fuel_bounds_the_cpu() {
        let err = js(
            "function main() { let n = 0; while (true) { n++; } }",
            "{}",
            Capabilities::default(),
            Limits {
                fuel: 50_000_000,
                timeout: Duration::from_secs(60),
                ..Limits::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, SandboxError::OutOfFuel);
    }

    /// Output handling: reads and writes past 1 MiB are refused.
    #[tokio::test]
    async fn reads_and_writes_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), vec![b'x'; (1 << 20) + 1]).unwrap();
        let caps = Capabilities {
            read_prefixes: vec!["".into()],
            write_prefixes: vec!["".into()],
            ..Default::default()
        };
        let workspace = || {
            Some(Workspace {
                root: dir.path().to_path_buf(),
                read: |p| std::fs::read(p),
                write: |p, b| std::fs::write(p, b),
            })
        };
        let (read, _) = run(
            "function main() { return read('big.txt').length; }",
            "{}",
            &caps,
            Limits::default(),
            workspace(),
        )
        .await;
        assert!(read.unwrap_err().to_string().contains("larger than"));
        let (wrote, _) = run(
            "function main() { write('out.txt', 'x'.repeat(1048577)); }",
            "{}",
            &caps,
            Limits::default(),
            workspace(),
        )
        .await;
        assert!(wrote.unwrap_err().to_string().contains("at most"));
        assert!(!dir.path().join("out.txt").exists());
    }

    /// Reads cannot leave the workspace through a link.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_read_cannot_follow_a_link_out_of_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "no").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        let (out, _) = run(
            "function main() { return read('link/secret.txt'); }",
            "{}",
            &Capabilities {
                read_prefixes: vec!["".into()],
                ..Default::default()
            },
            Limits::default(),
            Some(Workspace {
                root: dir.path().to_path_buf(),
                read: |p| std::fs::read(p),
                write: |p, b| std::fs::write(p, b),
            }),
        )
        .await;
        let err = out.unwrap_err().to_string();
        assert!(err.contains("leaves the workspace"), "{err}");
    }

    /// Credentials: only for the host they were granted for, and only when the value is known.
    #[test]
    fn a_credential_goes_only_to_its_host() {
        let caps = Capabilities {
            http_hosts: vec!["api.example.com".into(), "other.example.com".into()],
            credentials: vec!["api.example.com=EXAMPLE_TOKEN".into()],
            ..Default::default()
        };
        let secrets: std::collections::HashMap<String, String> =
            [("EXAMPLE_TOKEN".to_string(), "s3cret-value".to_string())].into();
        assert_eq!(
            credential_for(&caps, &secrets, "API.example.com"),
            Some("s3cret-value")
        );
        assert_eq!(credential_for(&caps, &secrets, "other.example.com"), None);
        assert_eq!(
            credential_for(&caps, &Default::default(), "api.example.com"),
            None
        );
        // Attached as a bearer token on the request itself.
        let request = pinned_client("api.example.com", "93.184.216.34:443".parse().unwrap())
            .unwrap()
            .get("https://api.example.com/")
            .bearer_auth(credential_for(&caps, &secrets, "api.example.com").unwrap())
            .build()
            .unwrap();
        assert_eq!(request.headers()["authorization"], "Bearer s3cret-value");
    }

    #[test]
    fn private_addresses_are_recognized() {
        for ip in [
            "10.0.0.1",
            "192.168.1.1",
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
            // Ranges that are not on the internet, and IPv4 inside IPv6.
            "0.1.2.3",
            "224.0.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "192.0.0.8",
            "ff02::1",
            "::10.0.0.1",
            "64:ff9b::a00:1",
            "2002:a00:1::",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700::1111",
            "64:ff9b::5db8:d822",
            "2002:5db8:d822::",
        ] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }
}
