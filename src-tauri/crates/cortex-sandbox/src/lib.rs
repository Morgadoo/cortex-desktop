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
                // Local-use NAT64 64:ff9b:1::/48, and documentation 2001:db8::/32
                || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1)
                || (s[0] == 0x2001 && s[1] == 0x0db8)
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
        let relative = plain_relative(&path)?;
        if !self
            .capabilities
            .read_prefixes
            .iter()
            .any(|prefix| prefix.is_empty() || relative.starts_with(prefix))
        {
            return Err(format!("denied: no read capability for `{path}`"));
        }
        let root = tokio::fs::canonicalize(&workspace)
            .await
            .map_err(|_| "the workspace is empty".to_string())?;
        // Opened by walking down from the workspace without following a link at any level, so
        // nothing swapped in between a check and the read can lead it outside.
        let bytes =
            tokio::task::spawn_blocking(move || beneath::read(&root, &relative, MAX_READ_BYTES))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.describe(&path, "read"))?;
        read(Path::new(&path), bytes).map_err(|_| format!("could not read `{path}`"))
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
        let relative = plain_relative(&path)?;
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
        let bytes =
            write(&root.join(&relative), &data).map_err(|_| format!("could not write `{path}`"))?;
        // Folders are made and the file opened one level at a time from the workspace, never
        // through a link: nothing is made or written outside, whatever changes meanwhile.
        tokio::task::spawn_blocking(move || beneath::write(&root, &relative, &bytes))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.describe(&path, "write"))
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
            push_capped(&mut body, &chunk, MAX_HTTP_BODY_BYTES)?;
        }
        Ok(Response { status, body })
    }
}

/// Adds a chunk of a response as it arrives, and stops the download the moment it passes `cap`:
/// a large response is refused without being held in memory.
fn push_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> Result<(), String> {
    if body.len() + chunk.len() > cap {
        return Err(format!("denied: the response is larger than {cap} bytes"));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// A script's path, if it is plain and relative: no root, no `..`, no `.`.
fn plain_relative(path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    if path.is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, PathPart::Normal(_)))
    {
        return Err(format!("denied: `{path}` is not a plain relative path"));
    }
    Ok(relative.to_path_buf())
}

/// Why a file in the workspace could not be reached.
#[derive(Debug)]
enum Reach {
    Link,
    NotFound,
    TooLarge,
    NotAFile,
    Io(std::io::Error),
}

impl Reach {
    fn describe(self, path: &str, verb: &str) -> String {
        match self {
            Reach::Link => format!(
                "denied: `{path}` goes through a link, which may lead outside the folders this tool may {verb}, or that leaves the workspace"
            ),
            Reach::NotFound => format!("not found: `{path}`"),
            Reach::TooLarge => format!("denied: `{path}` is larger than {MAX_READ_BYTES} bytes"),
            Reach::NotAFile => format!("denied: `{path}` is a folder, not a file"),
            Reach::Io(e) => format!("could not {verb} `{path}`: {e}"),
        }
    }
}

/// Files under the workspace, reached without following links: each folder is opened from the
/// one above it (`openat` with `O_NOFOLLOW`), so a link anywhere on the way, or one swapped in
/// mid-call, is refused rather than followed.
#[cfg(unix)]
mod beneath {
    use super::Reach;
    use std::ffi::CString;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    fn name(part: &std::ffi::OsStr) -> Result<CString, Reach> {
        CString::new(part.as_bytes()).map_err(|_| Reach::NotFound)
    }

    fn error() -> Reach {
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ELOOP) | Some(libc::ENOTDIR) => Reach::Link,
            Some(libc::ENOENT) => Reach::NotFound,
            Some(libc::EISDIR) => Reach::NotAFile,
            _ => Reach::Io(e),
        }
    }

    fn open_root(root: &Path) -> Result<OwnedFd, Reach> {
        let root = CString::new(root.as_os_str().as_bytes()).map_err(|_| Reach::NotFound)?;
        // SAFETY: a valid C string; the returned descriptor is owned below.
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(error());
        }
        // SAFETY: `fd` is a fresh descriptor nobody else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// The folder `parts` names under `dir`, made where missing when `create`.
    fn folder(dir: OwnedFd, parts: &[Component], create: bool) -> Result<OwnedFd, Reach> {
        let mut dir = dir;
        for part in parts {
            let Component::Normal(part) = part else {
                return Err(Reach::NotFound);
            };
            let part = name(part)?;
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            // SAFETY: valid descriptor and C string.
            let mut fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
            if fd < 0
                && create
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
            {
                // SAFETY: as above; mkdirat makes a folder, never follows a link.
                if unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o755) } != 0 {
                    return Err(error());
                }
                // SAFETY: as above.
                fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
            }
            if fd < 0 {
                return Err(error());
            }
            // SAFETY: `fd` is fresh and owned here.
            dir = unsafe { OwnedFd::from_raw_fd(fd) };
        }
        Ok(dir)
    }

    fn file(dir: &OwnedFd, part: &Component, flags: i32) -> Result<File, Reach> {
        let Component::Normal(part) = part else {
            return Err(Reach::NotFound);
        };
        let part = name(part)?;
        // SAFETY: valid descriptor and C string; O_NOFOLLOW refuses a link in the file's place.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                // O_NONBLOCK: a FIFO in the workspace returns at once instead of blocking a
                // thread forever; anything but a plain file is then refused.
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o644 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(error());
        }
        // SAFETY: `fd` is fresh and owned by the File.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub fn read(root: &Path, relative: &Path, max: u64) -> Result<Vec<u8>, Reach> {
        let parts: Vec<Component> = relative.components().collect();
        let (last, folders) = parts.split_last().ok_or(Reach::NotFound)?;
        let dir = folder(open_root(root)?, folders, false)?;
        let mut f = file(&dir, last, libc::O_RDONLY)?;
        let meta = f.metadata().map_err(Reach::Io)?;
        if !meta.is_file() {
            return Err(Reach::NotAFile);
        }
        // A hard link names a file that may live in a folder the tool was not granted.
        if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
            return Err(Reach::Link);
        }
        if meta.len() > max {
            return Err(Reach::TooLarge);
        }
        let mut out = Vec::new();
        Read::take(&mut f, max + 1)
            .read_to_end(&mut out)
            .map_err(Reach::Io)?;
        if out.len() as u64 > max {
            return Err(Reach::TooLarge);
        }
        Ok(out)
    }

    pub fn write(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), Reach> {
        let parts: Vec<Component> = relative.components().collect();
        let (last, folders) = parts.split_last().ok_or(Reach::NotFound)?;
        let dir = folder(open_root(root)?, folders, true)?;
        // Not truncated on open: a hard-linked file is refused before anything in it changes.
        let mut f = file(&dir, last, libc::O_WRONLY | libc::O_CREAT)?;
        let meta = f.metadata().map_err(Reach::Io)?;
        if !meta.is_file() {
            return Err(Reach::NotAFile);
        }
        if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
            return Err(Reach::Link);
        }
        f.set_len(0).map_err(Reach::Io)?;
        f.write_all(bytes).map_err(Reach::Io)
    }
}

/// Elsewhere, the same rules by path: each level checked for a link before it is used.
#[cfg(not(unix))]
mod beneath {
    use super::Reach;
    use std::path::Path;

    fn walk(root: &Path, relative: &Path, create: bool) -> Result<std::path::PathBuf, Reach> {
        let mut at = root.to_path_buf();
        let parts: Vec<_> = relative.components().collect();
        for (i, part) in parts.iter().enumerate() {
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Ok(m) if m.file_type().is_symlink() => return Err(Reach::Link),
                Ok(_) => {}
                Err(_) if create && i + 1 < parts.len() => {
                    std::fs::create_dir(&at).map_err(Reach::Io)?
                }
                Err(_) if create => {}
                Err(_) => return Err(Reach::NotFound),
            }
        }
        Ok(at)
    }

    pub fn read(root: &Path, relative: &Path, max: u64) -> Result<Vec<u8>, Reach> {
        let at = walk(root, relative, false)?;
        let meta = std::fs::metadata(&at).map_err(Reach::Io)?;
        if !meta.is_file() {
            return Err(Reach::NotAFile);
        }
        if meta.len() > max {
            return Err(Reach::TooLarge);
        }
        std::fs::read(at).map_err(Reach::Io)
    }

    pub fn write(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), Reach> {
        let at = walk(root, relative, true)?;
        if at.is_dir() {
            return Err(Reach::NotAFile);
        }
        std::fs::write(at, bytes).map_err(Reach::Io)
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

#[cfg(test)]
mod tests;
