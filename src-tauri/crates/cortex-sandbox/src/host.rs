//! What a running tool may ask of the host: credentials for allowed hosts, resource limits, and the WIT host interface (files, HTTP, logs, calls to other tools).

use super::*;

/// The secret to send to `host`: one the tool was granted for that host, whose value is known.
pub(super) fn credential_for<'a>(
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
pub(super) fn is_private(ip: IpAddr) -> bool {
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
