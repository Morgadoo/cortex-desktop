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

mod descriptions;
mod operations;
use descriptions::*;

#[cfg(test)]
mod tests;
