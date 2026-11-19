use crate::account::AccountId;
use crate::settings::load_account_settings;
use crate::tools::auth::{AuthMechanism, Readiness};
use crate::tools::connector_state::ToolPolicy;
use crate::tools::error::ToolError;
use crate::tools::icons::bundled_icon_data_uri;
use crate::tools::registry::{Connector, ConnectorDescriptor, ToolDefinition};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use glob::Pattern;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::RwLock;
use tauri::{AppHandle, Manager};
use walkdir::WalkDir;

const MAX_FILE_SIZE_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_MAX_RESULTS: usize = 50;
const CONTENT_CONTEXT_CHARS: usize = 500;

struct FilesystemCore {
    allowed_roots: RwLock<Vec<PathBuf>>,
    /// Cortex's own settings, credentials metadata and web storage, out of reach even inside an
    /// allowed folder, so injected instructions cannot repoint a connector or lift a session.
    protected: Vec<PathBuf>,
    write_enabled: RwLock<bool>,
    max_file_size_bytes: u64,
    max_results: usize,
}

impl FilesystemCore {
    fn new(allowed_roots: Vec<PathBuf>, protected: Vec<PathBuf>, write_enabled: bool) -> Self {
        Self {
            allowed_roots: RwLock::new(allowed_roots),
            protected,
            write_enabled: RwLock::new(write_enabled),
            max_file_size_bytes: MAX_FILE_SIZE_BYTES,
            max_results: DEFAULT_MAX_RESULTS,
        }
    }

    fn readiness(&self) -> Readiness {
        let roots = self
            .allowed_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if roots.is_empty() {
            Readiness::blocked("Add a folder to grant access")
        } else {
            Readiness::Ready
        }
    }

    /// Resolved per operation: a directory created after startup canonicalizes differently
    /// (on Windows, with a `\\?\` prefix) from the path it was configured as.
    fn protected_paths(&self) -> Vec<PathBuf> {
        self.protected
            .iter()
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
            .collect()
    }

    fn is_protected(protected: &[PathBuf], canonical: &Path) -> bool {
        protected.iter().any(|dir| canonical.starts_with(dir))
    }

    fn resolve_allowed_path(&self, path: &str) -> Result<PathBuf, ToolError> {
        self.resolve_within_roots(path, false)
    }

    fn resolve_allowed_path_for_create(&self, path: &str) -> Result<PathBuf, ToolError> {
        self.resolve_within_roots(path, true)
    }

    fn resolve_within_roots(
        &self,
        path: &str,
        allow_missing_leaf: bool,
    ) -> Result<PathBuf, ToolError> {
        let candidate = sanitize_user_path(path)?;
        let roots = self.allowed_roots.read().map_err(|_| {
            ToolError::AccessDenied("Filesystem permissions are unavailable.".into())
        })?;
        if roots.is_empty() {
            return Err(ToolError::AccessDenied(
                "no allowed directories configured".into(),
            ));
        }

        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            roots[0].join(candidate)
        };

        let leaf_is_symlink = std::fs::symlink_metadata(&resolved)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);

        let canonical = if allow_missing_leaf && !resolved.exists() && !leaf_is_symlink {
            let parent = resolved
                .parent()
                .ok_or_else(|| ToolError::InvalidInput("path has no parent directory".into()))?;
            let file_name = resolved
                .file_name()
                .ok_or_else(|| ToolError::InvalidInput("path has no file name".into()))?;
            parent
                .canonicalize()
                .map_err(|e| {
                    ToolError::InvalidInput(format!("parent directory does not exist: {e}"))
                })?
                .join(file_name)
        } else {
            resolved
                .canonicalize()
                .map_err(|e| ToolError::AccessDenied(format!("invalid path: {e}")))?
        };

        for root in roots.iter() {
            let canonical_root = root
                .canonicalize()
                .map_err(|e| ToolError::AccessDenied(format!("invalid root: {e}")))?;
            if canonical.starts_with(&canonical_root) {
                if Self::is_protected(&self.protected_paths(), &canonical) {
                    return Err(ToolError::AccessDenied(
                        "Cortex's own settings and data are not available to tools".into(),
                    ));
                }
                return Ok(canonical);
            }
        }

        Err(ToolError::AccessDenied(format!(
            "path outside allowed roots: {}",
            canonical.display()
        )))
    }

    fn search_roots(&self, root: Option<&str>) -> Result<Vec<PathBuf>, ToolError> {
        let roots = self.allowed_roots.read().map_err(|_| {
            ToolError::AccessDenied("Filesystem permissions are unavailable.".into())
        })?;
        if roots.is_empty() {
            return Err(ToolError::AccessDenied(
                "no allowed directories configured".into(),
            ));
        }

        if let Some(sub) = root {
            let resolved = self.resolve_allowed_path(sub)?;
            if !resolved.is_dir() {
                return Err(ToolError::InvalidInput(format!(
                    "search root is not a directory: {}",
                    resolved.display()
                )));
            }
            return Ok(vec![resolved]);
        }

        // Canonical, so the walk's paths can be compared with the protected ones.
        Ok(roots
            .iter()
            .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
            .collect())
    }

    fn allowed_roots_for_prompt(&self) -> Vec<String> {
        self.allowed_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|path| display_root_path(path.as_path()))
            .collect()
    }
}

pub struct FilesystemConnector {
    core: FilesystemCore,
}

impl FilesystemConnector {
    #[cfg(test)]
    pub fn new(allowed_roots: Vec<PathBuf>, write_enabled: bool) -> Self {
        Self::with_protected(allowed_roots, Vec::new(), write_enabled)
    }

    fn with_protected(
        allowed_roots: Vec<PathBuf>,
        protected: Vec<PathBuf>,
        write_enabled: bool,
    ) -> Self {
        Self {
            core: FilesystemCore::new(allowed_roots, protected, write_enabled),
        }
    }

    pub fn from_settings(app_handle: &AppHandle, account: &AccountId) -> Self {
        let (allowed_roots, write_enabled) = load_filesystem_settings(app_handle, account);
        Self::with_protected(allowed_roots, app_directories(app_handle), write_enabled)
    }
}

/// Every directory Cortex keeps state in, including the webview's cookies and storage.
fn app_directories(app_handle: &AppHandle) -> Vec<PathBuf> {
    let path = app_handle.path();
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut dirs: Vec<PathBuf> = [
        path.app_config_dir(),
        path.app_data_dir(),
        path.app_local_data_dir(),
        path.app_cache_dir(),
        path.app_log_dir(),
    ]
    .into_iter()
    .filter_map(Result::ok)
    .collect();
    // WebKit keeps a macOS app's web storage outside the app's own directories.
    #[cfg(target_os = "macos")]
    if let Ok(home) = path.home_dir() {
        let identifier = &app_handle.config().identifier;
        let library = home.join("Library");
        dirs.push(library.join("WebKit").join(identifier));
        dirs.push(library.join("HTTPStorages").join(identifier));
        dirs.push(
            library
                .join("Cookies")
                .join(format!("{identifier}.binarycookies")),
        );
    }
    dirs
}

fn display_root_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn format_allowed_roots_block(roots: &[String]) -> String {
    if roots.is_empty() {
        return "No allowed directories are configured yet.".into();
    }
    let listed = roots
        .iter()
        .map(|root| format!("- {root}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Allowed directories:\n{listed}")
}

fn fs_search_description(roots: &[String]) -> String {
    format!(
        "Search for files by name pattern or content. {}\n\
         Use `root` to limit the search to a subdirectory of one allowed root. \
         Omit `root` to search all allowed directories. \
         Paths may be absolute (under an allowed root) or relative to the first allowed root.",
        format_allowed_roots_block(roots)
    )
}

fn fs_read_description(roots: &[String]) -> String {
    format!(
        "Open and read a local file on the user's computer, including files found by fs_search. \
         Use this tool for local files, rather than knowledge-base lookup or analysis tools, which cannot open local filesystem paths. \
         Supports text, code, PDF, documents, CSV, spreadsheets and images, up to 5 MiB per file. \
         Set `format` to `file` and omit `start` and `end` for whole-file processing. \
         Returns readable text or image content with an image_id, depending on the file type. \
         Local files do not need to be uploaded to the chat first. \
         Byte ranges and `format: raw` return raw content and should be used only for text files. {}\n\
         The `path` must be absolute under one of these roots, or relative to the first allowed root.",
        format_allowed_roots_block(roots)
    )
}

fn fs_search_root_property_description(roots: &[String]) -> String {
    if roots.is_empty() {
        return "Sub-path within an allowed root. Omit to search all roots.".into();
    }
    format!(
        "Optional subdirectory to search within. Must be under one of: {}. Omit to search all allowed directories.",
        roots.join(", ")
    )
}

fn fs_read_path_property_description(roots: &[String]) -> String {
    if roots.is_empty() {
        return "File path under an allowed root.".into();
    }
    format!(
        "File path under one of the allowed directories: {}. \
         Absolute paths must stay within an allowed root; otherwise they are resolved relative to {}.",
        roots.join(", "),
        roots[0]
    )
}

fn fs_write_description(roots: &[String]) -> String {
    format!(
        "Create or overwrite a text file under an allowed directory. {}\n\
         Existing files are replaced entirely. Parent directories must already exist.",
        format_allowed_roots_block(roots)
    )
}

fn fs_edit_description(roots: &[String]) -> String {
    format!(
        "Edit a text file by replacing strings. {}\n\
         Fails if `old_string` is not found unless `replace_all` is true and every occurrence is replaced.",
        format_allowed_roots_block(roots)
    )
}

fn load_filesystem_settings(app_handle: &AppHandle, account: &AccountId) -> (Vec<PathBuf>, bool) {
    let settings = load_account_settings(app_handle, account).unwrap_or_default();
    let roots = settings
        .filesystem_allowed_roots
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    (roots, settings.filesystem_write_enabled)
}

fn is_binary(data: &[u8]) -> bool {
    data.iter().take(512).any(|&b| b == 0)
}

fn image_mime_type(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if header.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if header.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// Per character, so it lines up with `find_ignoring_case`; `str::to_lowercase` treats a final
/// sigma differently.
fn lowercase(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Byte offset in `line` of the first case-insensitive match of `needle`, which must already be
/// lowercase. Lowercasing can change a character's length (`İ` becomes `i̇`), so an offset into the
/// lowercased copy can't index `line` directly.
fn find_ignoring_case(line: &str, needle: &str) -> Option<usize> {
    let mut lowered = String::with_capacity(line.len());
    let mut origins = Vec::with_capacity(line.len());
    for (offset, c) in line.char_indices() {
        lowered.extend(c.to_lowercase());
        origins.resize(lowered.len(), offset);
    }
    lowered
        .find(needle)
        .map(|at| origins.get(at).copied().unwrap_or(line.len()))
}

/// About `CONTENT_CONTEXT_CHARS` bytes of `line` around `start`, cut on character boundaries.
fn snippet_around(line: &str, start: usize) -> &str {
    let from = line.floor_char_boundary(start.saturating_sub(CONTENT_CONTEXT_CHARS / 2));
    let to = line.ceil_char_boundary((from + CONTENT_CONTEXT_CHARS).min(line.len()));
    &line[from..to]
}

/// Refuses `..`, and any path that isn't on a local drive. Must run before the path reaches the
/// OS: on Windows, merely resolving `\\host\share` connects to the host over SMB and offers the
/// user's NTLM credentials, and the host name lookup alone carries data out.
pub(super) fn sanitize_user_path(path: &str) -> Result<PathBuf, ToolError> {
    let remote = || ToolError::AccessDenied("network and device paths are not allowed".into());
    let path = Path::new(path);
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(ToolError::AccessDenied("path traversal not allowed".into()))
            }
            // `\\?\C:\` is how canonical paths come back on Windows, so it has to stay usable.
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => {}
                _ => return Err(remote()),
            },
            Component::RootDir | Component::Normal(_) | Component::CurDir => {}
        }
    }
    // Windows reads `\\host` and `//host` as shares whatever the platform parser above saw.
    let text = path.as_os_str().to_string_lossy();
    let is_separator = |c: char| matches!(c, '/' | '\\');
    let starts_like_a_share = text.starts_with(is_separator) && text[1..].starts_with(is_separator);
    let has_disk_prefix = matches!(path.components().next(), Some(Component::Prefix(_)));
    if starts_like_a_share && !has_disk_prefix {
        return Err(remote());
    }
    Ok(path.to_path_buf())
}

#[async_trait]
impl Connector for FilesystemConnector {
    fn id(&self) -> &str {
        "filesystem"
    }

    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            id: "filesystem".into(),
            display_name: "Filesystem".into(),
            description: None,
            version: "1.0.0".into(),
            icon: bundled_icon_data_uri("filesystem"),
            auth: AuthMechanism::None,
        }
    }

    fn readiness(&self) -> Readiness {
        self.core.readiness()
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        let roots = self.core.allowed_roots_for_prompt();
        let write_enabled = *self
            .core
            .write_enabled
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut tools = vec![
            ToolDefinition {
                name: "fs_search".into(),
                read_only: Some(true),
                title: Some("Search Files".into()),
                description: fs_search_description(&roots),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "root": {
                            "type": "string",
                            "description": fs_search_root_property_description(&roots),
                        },
                        "mode": { "type": "string", "enum": ["name", "content"], "default": "name" },
                        "max_results": { "type": "integer", "default": 20 }
                    },
                    "required": ["query"]
                }),
                ..Default::default()
            },
            ToolDefinition {
                name: "fs_read".into(),
                read_only: Some(true),
                title: Some("Read File".into()),
                description: fs_read_description(&roots),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": fs_read_path_property_description(&roots),
                        },
                        "start": { "type": "integer" },
                        "end": { "type": "integer" },
                        "format": {
                            "type": "string",
                            "enum": ["raw", "file"],
                            "default": "raw",
                            "description": "Use file for whole-file processing. Use raw only for text or byte-range reads."
                        }
                    },
                    "required": ["path"]
                }),
                ..Default::default()
            },
        ];

        if write_enabled {
            tools.push(ToolDefinition {
                name: "fs_write".into(),
                read_only: Some(false),
                open_world: None,
                policy: ToolPolicy::Ask,
                title: Some("Write File".into()),
                description: fs_write_description(&roots),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": fs_read_path_property_description(&roots),
                        },
                        "content": {
                            "type": "string",
                            "description": "Full file contents to write"
                        }
                    },
                    "required": ["path", "content"]
                }),
            });
            tools.push(ToolDefinition {
                name: "fs_edit".into(),
                read_only: Some(false),
                open_world: None,
                policy: ToolPolicy::Ask,
                title: Some("Edit File".into()),
                description: fs_edit_description(&roots),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": fs_read_path_property_description(&roots),
                        },
                        "old_string": {
                            "type": "string",
                            "description": "Text to find in the file"
                        },
                        "new_string": {
                            "type": "string",
                            "description": "Replacement text"
                        },
                        "replace_all": {
                            "type": "boolean",
                            "default": false,
                            "description": "Replace every occurrence instead of just the first"
                        }
                    },
                    "required": ["path", "old_string", "new_string"]
                }),
            });
        }

        tools
    }

    async fn invoke(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        match tool_name {
            "fs_search" => self.core.fs_search(input).await,
            "fs_read" => self.core.fs_read(input).await,
            "fs_write" => self.core.fs_write(input).await,
            "fs_edit" => self.core.fs_edit(input).await,
            _ => Err(ToolError::UnknownTool(tool_name.into())),
        }
    }
}

impl FilesystemCore {
    async fn fs_search(&self, input: Value) -> Result<Value, ToolError> {
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("query is required".into()))?;

        let mode = input.get("mode").and_then(Value::as_str).unwrap_or("name");

        let max_results = input
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(20)
            .min(self.max_results);

        let root = input.get("root").and_then(Value::as_str);
        let search_roots = self.search_roots(root)?;
        let protected = self.protected_paths();

        let mut results = Vec::new();

        match mode {
            "name" => {
                let pattern = if query.contains('*') || query.contains('?') {
                    query.to_string()
                } else {
                    format!("*{query}*")
                };
                let glob = Pattern::new(&pattern)
                    .map_err(|e| ToolError::InvalidInput(format!("invalid glob pattern: {e}")))?;

                'outer: for root in search_roots {
                    for entry in WalkDir::new(&root)
                        .into_iter()
                        .filter_entry(|e| !FilesystemCore::is_protected(&protected, e.path()))
                        .filter_map(|e| e.ok())
                    {
                        if results.len() >= max_results {
                            break 'outer;
                        }
                        let path = entry.path();
                        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if glob.matches(file_name) {
                            results.push(json!({
                                "path": path.display().to_string(),
                                "name": file_name,
                            }));
                        }
                    }
                }
            }
            "content" => {
                let needle = lowercase(query);
                'outer: for root in search_roots {
                    for entry in WalkDir::new(&root)
                        .into_iter()
                        .filter_entry(|e| !FilesystemCore::is_protected(&protected, e.path()))
                        .filter_map(|e| e.ok())
                    {
                        if results.len() >= max_results {
                            break 'outer;
                        }
                        // A symlink can point outside the root, and its contents would leak
                        // through the snippet without the root check fs_read applies.
                        if entry.path_is_symlink() {
                            continue;
                        }
                        let path = entry.path();
                        if !path.is_file() {
                            continue;
                        }
                        if let Ok(meta) = path.metadata() {
                            if meta.len() > self.max_file_size_bytes {
                                continue;
                            }
                        }
                        if let Ok(file) = std::fs::File::open(path) {
                            let mut reader = BufReader::new(file);
                            let mut header = vec![0u8; 512];
                            let n = reader.read(&mut header).unwrap_or(0);
                            if is_binary(&header[..n]) {
                                continue;
                            }
                            reader.seek(SeekFrom::Start(0)).ok();
                            for (line_no, line) in reader.lines().enumerate() {
                                if results.len() >= max_results {
                                    break 'outer;
                                }
                                // Not UTF-8 (Latin-1 and the like): skip the file, not the search.
                                let Ok(line) = line else { break };
                                if let Some(start) = find_ignoring_case(&line, &needle) {
                                    results.push(json!({
                                        "path": path.display().to_string(),
                                        "line": line_no + 1,
                                        "snippet": snippet_around(&line, start),
                                    }));
                                }
                            }
                        }
                    }
                }
            }
            other => {
                return Err(ToolError::InvalidInput(format!(
                    "unsupported search mode: {other}"
                )));
            }
        }

        Ok(json!({ "results": results, "count": results.len() }))
    }

    async fn fs_read(&self, input: Value) -> Result<Value, ToolError> {
        let path_str = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path is required".into()))?;

        sanitize_user_path(path_str)?;
        let path = self.resolve_allowed_path(path_str)?;

        if !path.is_file() {
            return Err(ToolError::InvalidInput(format!(
                "not a file: {}",
                path.display()
            )));
        }

        let meta = path
            .metadata()
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let start = input.get("start").and_then(Value::as_u64).unwrap_or(0);
        let end = input
            .get("end")
            .and_then(Value::as_u64)
            .unwrap_or(meta.len());

        if end < start {
            return Err(ToolError::InvalidInput("end must be >= start".into()));
        }

        let mut file =
            std::fs::File::open(&path).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let mut header = [0; 16];
        let header_len = file
            .read(&mut header)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let mime_type = image_mime_type(&header[..header_len]);
        let format = input.get("format").and_then(Value::as_str).unwrap_or("raw");
        if !matches!(format, "raw" | "file") {
            return Err(ToolError::InvalidInput("format must be raw or file".into()));
        }
        if format == "file" {
            if start != 0 || end != meta.len() {
                return Err(ToolError::InvalidInput(
                    "File processing requires a whole file; omit start and end.".into(),
                ));
            }
            if meta.len() > self.max_file_size_bytes {
                return Err(ToolError::InvalidInput(
                    "File exceeds the 5 MiB desktop read limit.".into(),
                ));
            }
            file.rewind()
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            let mut bytes = Vec::new();
            file.take(self.max_file_size_bytes + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            if bytes.len() as u64 > self.max_file_size_bytes {
                return Err(ToolError::InvalidInput(
                    "File grew beyond the 5 MiB limit.".into(),
                ));
            }
            // Transport only: the client's attachment service owns format detection
            // and conversion. Signature hints also support extensionless images/PDFs.
            let mime_type = mime_type.unwrap_or(if bytes.starts_with(b"%PDF-") {
                "application/pdf"
            } else {
                "application/octet-stream"
            });
            return Ok(json!({
                "type": "cortex_file",
                "file_id": uuid::Uuid::new_v4().to_string(),
                "path": path.display().to_string(),
                "name": path.file_name().unwrap_or_default().to_string_lossy(),
                "size": bytes.len(),
                "mime_type": mime_type,
                "binary": is_binary(&bytes) || std::str::from_utf8(&bytes).is_err(),
                "data": STANDARD.encode(&bytes),
            }));
        }
        file.seek(SeekFrom::Start(start))
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let to_read = (end - start).min(self.max_file_size_bytes) as usize;
        let mut buffer = Vec::new();
        let read = file
            .take(to_read as u64)
            .read_to_end(&mut buffer)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let binary = is_binary(&buffer);
        let content = if binary {
            json!({
                "encoding": "base64",
                "content": STANDARD.encode(&buffer),
            })
        } else {
            json!({
                "encoding": "utf-8",
                "content": String::from_utf8_lossy(&buffer).to_string(),
            })
        };

        Ok(json!({
            "path": path.display().to_string(),
            "size": meta.len(),
            "start": start,
            "end": start + read as u64,
            "binary": binary,
            "content": content,
        }))
    }

    async fn fs_write(&self, input: Value) -> Result<Value, ToolError> {
        if !*self.write_enabled.read().map_err(|_| {
            ToolError::AccessDenied("Filesystem write permissions are unavailable.".into())
        })? {
            return Err(ToolError::AccessDenied(
                "file editing is disabled in Cortex settings".into(),
            ));
        }

        let path_str = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path is required".into()))?;
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("content is required".into()))?;

        if content.len() as u64 > self.max_file_size_bytes {
            return Err(ToolError::InvalidInput(format!(
                "content exceeds {} byte limit",
                self.max_file_size_bytes
            )));
        }

        sanitize_user_path(path_str)?;
        let path = self.resolve_allowed_path_for_create(path_str)?;

        if path.exists() && !path.is_file() {
            return Err(ToolError::InvalidInput(format!(
                "not a file: {}",
                path.display()
            )));
        }

        if let Some(parent) = path.parent() {
            if !parent.is_dir() {
                return Err(ToolError::InvalidInput(format!(
                    "parent directory does not exist: {}",
                    parent.display()
                )));
            }
        }

        let created = !path.exists();
        // The text it replaces, so the result can say what changed.
        let previous = if created {
            None
        } else {
            std::fs::read_to_string(&path).ok()
        };
        std::fs::write(&path, content).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let mut result = json!({
            "path": path.display().to_string(),
            "bytes_written": content.len(),
            "created": created,
        });
        if let Some(previous) = previous {
            result["diff"] = json!(compact_diff(&previous, content));
        }
        Ok(result)
    }

    async fn fs_edit(&self, input: Value) -> Result<Value, ToolError> {
        if !*self.write_enabled.read().map_err(|_| {
            ToolError::AccessDenied("Filesystem write permissions are unavailable.".into())
        })? {
            return Err(ToolError::AccessDenied(
                "file editing is disabled in Cortex settings".into(),
            ));
        }

        let path_str = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path is required".into()))?;
        let old_string = input
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("old_string is required".into()))?;
        let new_string = input
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("new_string is required".into()))?;
        let replace_all = input
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        sanitize_user_path(path_str)?;
        let path = self.resolve_allowed_path(path_str)?;

        if !path.is_file() {
            return Err(ToolError::InvalidInput(format!(
                "not a file: {}",
                path.display()
            )));
        }

        let meta = path
            .metadata()
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        if meta.len() > self.max_file_size_bytes {
            return Err(ToolError::InvalidInput(format!(
                "file exceeds {} byte limit",
                self.max_file_size_bytes
            )));
        }

        let original = std::fs::read_to_string(&path)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let (updated, replacements) = if replace_all {
            if !original.contains(old_string) {
                return Err(ToolError::InvalidInput(
                    "old_string was not found in file".into(),
                ));
            }
            let count = original.matches(old_string).count();
            (original.replace(old_string, new_string), count)
        } else if let Some(index) = original.find(old_string) {
            let mut edited = original.clone();
            edited.replace_range(index..index + old_string.len(), new_string);
            (edited, 1)
        } else {
            return Err(ToolError::InvalidInput(
                "old_string was not found in file".into(),
            ));
        };

        if updated.len() as u64 > self.max_file_size_bytes {
            return Err(ToolError::InvalidInput(format!(
                "edited content exceeds {} byte limit",
                self.max_file_size_bytes
            )));
        }

        std::fs::write(&path, &updated).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(json!({
            "path": path.display().to_string(),
            "replacements": replacements,
            "bytes_written": updated.len(),
        }))
    }
}

/// Characters of diff an overwrite reports.
const DIFF_CHARS: usize = 6_000;

/// A unified diff of an overwrite, cut to a size worth showing, or "unchanged".
fn compact_diff(before: &str, after: &str) -> String {
    if before == after {
        return "unchanged".into();
    }
    let diff = similar::TextDiff::from_lines(before, after);
    let text = diff.unified_diff().context_radius(2).to_string();
    if text.chars().count() <= DIFF_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(DIFF_CHARS).collect();
    cut.push_str("\n… (diff cut)");
    cut
}

#[cfg(test)]
mod tests {
    #[test]
    fn overwrites_report_a_compact_diff() {
        let diff = super::compact_diff("a\nb\nc\n", "a\nB\nc\n");
        assert!(diff.contains("-b") && diff.contains("+B"), "{diff}");
        assert_eq!(super::compact_diff("x", "x"), "unchanged");
    }

    use super::*;
    use serde_json::json;
    use tempfile::{tempdir, TempDir};

    /// The kernel's guardrail: the agent cannot change its evaluator, its permissions or its audit
    /// log. They live in Cortex's own folder, which tools cannot reach even under an allowed root.
    #[tokio::test]
    async fn the_eval_sets_traces_and_audit_log_are_out_of_reach() {
        let home = tempdir().unwrap();
        let config = home.path().join(".config").join("cortex");
        for dir in ["evals/tool-selection/cases", "chat/audit"] {
            std::fs::create_dir_all(config.join(dir)).unwrap();
        }
        std::fs::write(config.join("chat/traces.db"), b"").unwrap();
        std::fs::write(config.join("connector_state.json"), b"{}").unwrap();
        let core = FilesystemCore::new(
            vec![home.path().to_path_buf()],
            vec![config.canonicalize().unwrap()],
            true,
        );
        for target in [
            "evals/tool-selection/cases/new.json",
            "chat/traces.db",
            "chat/audit/2026-11.jsonl",
            "connector_state.json",
        ] {
            let path = config.join(target);
            let written = core
                .fs_write(json!({ "path": path.to_string_lossy(), "content": "tampered" }))
                .await;
            assert!(
                matches!(&written, Err(ToolError::AccessDenied(m)) if m.contains("not available to tools")),
                "{target}: {written:?}"
            );
        }
        // Elsewhere under the same root, writing works.
        let ok = core
            .fs_write(
                json!({ "path": home.path().join("notes.txt").to_string_lossy(), "content": "x" }),
            )
            .await;
        assert!(ok.is_ok(), "{ok:?}");
    }

    fn core_rooted_at_temp_dir(write_enabled: bool) -> (TempDir, FilesystemCore) {
        let root = tempdir().unwrap();
        let core = FilesystemCore::new(vec![root.path().to_path_buf()], vec![], write_enabled);
        (root, core)
    }

    #[tokio::test]
    async fn cortex_own_directories_stay_out_of_reach_inside_an_allowed_folder() {
        let root = tempdir().unwrap();
        let app_dir = root
            .path()
            .join("Application Support")
            .join("io.github.morgadoo.cortex-desktop");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("mcp_servers.json"), "{\"servers\":[]}").unwrap();
        std::fs::write(root.path().join("notes.txt"), "servers").unwrap();
        let core =
            FilesystemCore::new(vec![root.path().to_path_buf()], vec![app_dir.clone()], true);
        let config = app_dir
            .join("mcp_servers.json")
            .to_string_lossy()
            .into_owned();

        assert!(matches!(
            core.fs_read(json!({ "path": config })).await,
            Err(ToolError::AccessDenied(_))
        ));
        assert!(matches!(
            core.fs_write(json!({ "path": config, "content": "{}" }))
                .await,
            Err(ToolError::AccessDenied(_))
        ));
        let created = app_dir.join("new.json").to_string_lossy().into_owned();
        assert!(matches!(
            core.fs_write(json!({ "path": created, "content": "{}" }))
                .await,
            Err(ToolError::AccessDenied(_))
        ));

        for mode in ["name", "content"] {
            let found = core
                .fs_search(json!({ "query": "servers", "mode": mode }))
                .await
                .unwrap();
            let text = found.to_string();
            assert!(!text.contains("mcp_servers.json"), "{mode}: {text}");
        }
        assert!(core
            .fs_search(json!({ "query": "notes", "mode": "name" }))
            .await
            .unwrap()
            .to_string()
            .contains("notes.txt"));
    }

    fn create_file(root: &TempDir, file_name: &str, contents: &str) -> String {
        let path = root.path().join(file_name);
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn path_in(root: &TempDir, file_name: &str) -> String {
        root.path().join(file_name).to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn reading_a_file_returns_its_contents() {
        let (root, core) = core_rooted_at_temp_dir(false);
        let file = create_file(&root, "readable_document.txt", "hello world");
        let result = core.fs_read(json!({ "path": file })).await.unwrap();
        assert_eq!(result["content"]["content"], json!("hello world"));
    }

    #[tokio::test]
    async fn whole_file_transport_is_format_agnostic_and_lossless() {
        let (root, core) = core_rooted_at_temp_dir(false);
        for (name, bytes) in [
            ("data.csv", b"name,value\nalice,1\n".to_vec()),
            ("sheet.xlsx", b"PK\x03\x04binary\0workbook".to_vec()),
            ("document.pdf", b"%PDF-1.4 opaque PDF bytes".to_vec()),
            ("document.docx", b"PK\x03\x04binary\0document".to_vec()),
            (
                "photo.data",
                [b"\x89PNG\r\n\x1a\n".as_slice(), &vec![42; 40_000]].concat(),
            ),
        ] {
            let path = root.path().join(name);
            std::fs::write(&path, &bytes).unwrap();
            let result = core
                .fs_read(json!({"path":path,"format":"file"}))
                .await
                .unwrap();
            assert_eq!(result["type"], "cortex_file");
            assert_eq!(result["name"], name);
            assert_eq!(result["size"], bytes.len());
            assert_eq!(
                STANDARD.decode(result["data"].as_str().unwrap()).unwrap(),
                bytes
            );
            assert!(core
                .fs_read(json!({"path":path,"format":"file","start":1}))
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn file_transport_enforces_size_and_folder_permissions() {
        let (root, core) = core_rooted_at_temp_dir(false);
        let path = root.path().join("large.csv");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_SIZE_BYTES + 1)
            .unwrap();
        assert!(core
            .fs_read(json!({"path":path,"format":"file"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("5 MiB"));
        let outside = tempdir().unwrap();
        let path = outside.path().join("secret.pdf");
        std::fs::write(&path, "secret").unwrap();
        assert!(matches!(
            core.fs_read(json!({"path":path,"format":"file"})).await,
            Err(ToolError::AccessDenied(_))
        ));
    }

    #[test]
    fn recognizes_supported_image_signatures() {
        for (header, mime) in [
            (b"\x89PNG\r\n\x1a\n".as_slice(), "image/png"),
            (b"\xff\xd8\xff".as_slice(), "image/jpeg"),
            (b"GIF89a".as_slice(), "image/gif"),
            (b"RIFF1234WEBP".as_slice(), "image/webp"),
            (b"BM".as_slice(), "image/bmp"),
        ] {
            assert_eq!(image_mime_type(header), Some(mime));
        }
        assert_eq!(image_mime_type(b"RIFF1234WAVE"), None);
        assert_eq!(image_mime_type(b"ordinary text"), None);
    }

    #[tokio::test]
    async fn writing_a_new_file_under_the_root_creates_it_on_disk() {
        let (root, core) = core_rooted_at_temp_dir(true);
        let path = path_in(&root, "created_by_write.txt");
        core.fs_write(json!({ "path": path, "content": "fresh contents" }))
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(root.path().join("created_by_write.txt")).unwrap();
        assert_eq!(on_disk, "fresh contents");
    }

    #[tokio::test]
    async fn writing_a_new_file_outside_the_root_is_denied() {
        let (_root, core) = core_rooted_at_temp_dir(true);
        let outside_root = tempdir().unwrap();
        let target = outside_root.path().join("created_outside_root.txt");
        let error = core
            .fs_write(json!({ "path": target.to_string_lossy(), "content": "escaped" }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::AccessDenied(_)));
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writing_a_new_file_through_a_symlink_that_escapes_the_root_is_denied() {
        let (root, core) = core_rooted_at_temp_dir(true);
        let outside_root = tempdir().unwrap();
        let link_inside_root = root.path().join("link_escaping_root.txt");
        std::os::unix::fs::symlink(outside_root.path().join("smuggled.txt"), &link_inside_root)
            .unwrap();
        let error = core
            .fs_write(json!({ "path": link_inside_root.to_string_lossy(), "content": "smuggled" }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::AccessDenied(_)));
        assert!(!outside_root.path().join("smuggled.txt").exists());
    }

    #[tokio::test]
    async fn writing_over_an_existing_file_replaces_its_contents() {
        let (root, core) = core_rooted_at_temp_dir(true);
        create_file(&root, "existing_document.txt", "old contents");
        core.fs_write(json!({
            "path": path_in(&root, "existing_document.txt"),
            "content": "new contents"
        }))
        .await
        .unwrap();
        let on_disk = std::fs::read_to_string(root.path().join("existing_document.txt")).unwrap();
        assert_eq!(on_disk, "new contents");
    }

    #[tokio::test]
    async fn editing_a_file_replaces_matched_text_on_disk() {
        let (root, core) = core_rooted_at_temp_dir(true);
        create_file(&root, "editable_document.txt", "the quick brown fox");
        core.fs_edit(json!({
            "path": path_in(&root, "editable_document.txt"),
            "old_string": "quick",
            "new_string": "slow"
        }))
        .await
        .unwrap();
        let on_disk = std::fs::read_to_string(root.path().join("editable_document.txt")).unwrap();
        assert_eq!(on_disk, "the slow brown fox");
    }

    #[tokio::test]
    async fn reading_a_parent_traversal_path_is_denied() {
        let (_root, core) = core_rooted_at_temp_dir(false);
        let error = core
            .fs_read(json!({ "path": "../outside_the_root.txt" }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::AccessDenied(_)));
    }

    #[test]
    fn a_match_after_text_that_grows_when_lowercased_is_found_in_the_original_line() {
        let line = format!("{}needle here", "İ".repeat(10));
        let start = find_ignoring_case(&line, "needle").unwrap();
        assert_eq!(&line[start..start + 6], "needle");
        assert_eq!(find_ignoring_case("Straße NEEDLE", "needle"), Some(8));
        assert_eq!(find_ignoring_case("ΟΔΟΣ", &lowercase("ΟΔΟΣ")), Some(0));
        assert_eq!(find_ignoring_case("nothing", "needle"), None);
    }

    #[test]
    fn snippets_are_cut_on_character_boundaries() {
        for filler in ["é", "日本", "İ", "🦀"] {
            let line = format!(
                "{}needle{}",
                filler.repeat(CONTENT_CONTEXT_CHARS),
                filler.repeat(CONTENT_CONTEXT_CHARS)
            );
            let found = find_ignoring_case(&line, "needle").unwrap();
            assert!(snippet_around(&line, found).contains("needle"), "{filler}");
            for start in found.saturating_sub(CONTENT_CONTEXT_CHARS)..line.len() {
                snippet_around(&line, start);
            }
        }
        assert_eq!(snippet_around("", 0), "");
    }

    #[tokio::test]
    async fn content_search_over_non_ascii_and_non_utf8_files_returns_matches() {
        let (root, core) = core_rooted_at_temp_dir(false);
        let accented = format!("{}NEEDLE{}", "İé日".repeat(200), "🦀".repeat(200));
        create_file(&root, "accented.txt", &accented);
        std::fs::write(root.path().join("latin1.txt"), b"caf\xe9 needle\n").unwrap();

        let found = core
            .fs_search(json!({ "query": "needle", "mode": "content" }))
            .await
            .unwrap()
            .to_string();
        assert!(found.contains("accented.txt"), "{found}");
        assert!(found.contains("NEEDLE"), "{found}");
    }

    const SHARES_AND_DEVICES: [&str; 7] = [
        r"\\attacker.example\share\x",
        "//attacker.example/share/x",
        r"\/attacker.example/share/x",
        r"\\?\UNC\attacker.example\share\x",
        r"\\.\pipe\x",
        r"\\?\GLOBALROOT\Device\Mup\attacker.example\share\x",
        r"\\attacker.example\share",
    ];

    fn refused_as_remote(result: Result<impl std::fmt::Debug, ToolError>) -> bool {
        matches!(result, Err(ToolError::AccessDenied(message)) if message.contains("network"))
    }

    #[tokio::test]
    async fn network_and_device_paths_are_refused_before_the_os_sees_them() {
        let (_root, core) = core_rooted_at_temp_dir(true);
        for path in SHARES_AND_DEVICES {
            assert!(refused_as_remote(sanitize_user_path(path)), "{path}");
            assert!(
                refused_as_remote(core.fs_read(json!({ "path": path })).await),
                "{path}"
            );
            assert!(
                refused_as_remote(core.fs_write(json!({ "path": path, "content": "x" })).await),
                "{path}"
            );
            assert!(
                refused_as_remote(
                    core.fs_edit(json!({ "path": path, "old_string": "a", "new_string": "b" }))
                        .await
                ),
                "{path}"
            );
            assert!(
                refused_as_remote(core.fs_search(json!({ "query": "x", "root": path })).await),
                "{path}"
            );
        }
    }

    #[test]
    fn local_paths_still_pass_the_path_check() {
        assert!(sanitize_user_path("notes/today.txt").is_ok());
        assert!(sanitize_user_path("/Users/me/notes.txt").is_ok());
        if cfg!(windows) {
            assert!(sanitize_user_path(r"C:\Users\me\notes.txt").is_ok());
            assert!(sanitize_user_path(r"\\?\C:\Users\me\notes.txt").is_ok());
        }
    }

    #[tokio::test]
    async fn reading_an_absolute_path_outside_the_root_is_denied() {
        let (_root, core) = core_rooted_at_temp_dir(false);
        let outside_root = tempdir().unwrap();
        let secret = outside_root.path().join("secret_outside_root.txt");
        std::fs::write(&secret, "confidential").unwrap();
        let error = core
            .fs_read(json!({ "path": secret.to_string_lossy() }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::AccessDenied(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reading_through_a_symlink_that_escapes_the_root_is_denied() {
        let (root, core) = core_rooted_at_temp_dir(false);
        let outside_root = tempdir().unwrap();
        let secret = outside_root.path().join("secret_outside_root.txt");
        std::fs::write(&secret, "confidential").unwrap();
        let link_inside_root = root.path().join("link_escaping_root.txt");
        std::os::unix::fs::symlink(&secret, &link_inside_root).unwrap();
        let error = core
            .fs_read(json!({ "path": link_inside_root.to_string_lossy() }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::AccessDenied(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn content_search_skips_symlinks_that_escape_the_root() {
        let (root, core) = core_rooted_at_temp_dir(false);
        let outside_root = tempdir().unwrap();
        let secret = outside_root.path().join("secret_outside_root.txt");
        std::fs::write(&secret, "needle confidential").unwrap();
        std::os::unix::fs::symlink(&secret, root.path().join("link_escaping_root.txt")).unwrap();
        create_file(&root, "inside_root.txt", "needle visible");
        let result = core
            .fs_search(json!({ "query": "needle", "mode": "content" }))
            .await
            .unwrap()
            .to_string();
        assert!(result.contains("visible"));
        assert!(!result.contains("confidential"));
    }

    #[tokio::test]
    async fn writing_is_denied_when_write_access_is_disabled() {
        let (root, core) = core_rooted_at_temp_dir(false);
        let error = core
            .fs_write(json!({
                "path": path_in(&root, "should_not_be_created.txt"),
                "content": "blocked"
            }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::AccessDenied(_)));
        assert!(!root.path().join("should_not_be_created.txt").exists());
    }
}
