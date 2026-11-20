//! The filesystem tools themselves: search, read, write and edit within the allowed folders.

use super::*;

impl FilesystemCore {
    pub(super) async fn fs_search(&self, input: Value) -> Result<Value, ToolError> {
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

    pub(super) async fn fs_read(&self, input: Value) -> Result<Value, ToolError> {
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

    pub(super) async fn fs_write(&self, input: Value) -> Result<Value, ToolError> {
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

    pub(super) async fn fs_edit(&self, input: Value) -> Result<Value, ToolError> {
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
