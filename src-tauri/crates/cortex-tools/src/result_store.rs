//! Bounded model previews with session-local, account-scoped read-only retrieval.
use super::error::ToolError;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

pub const MAX_PREVIEW_BYTES: usize = 32_000;
pub const MAX_PAGE_BYTES: usize = 16_000;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_INLINE_DATA_BYTES: usize = 5 * 1024 * 1024;
const MAX_INLINE_BASE64_LEN: usize = MAX_INLINE_DATA_BYTES.div_ceil(3) * 4;

pub struct ResultStore {
    state: Mutex<State>,
    capacity: usize,
}
impl Default for ResultStore {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            capacity: MAX_CACHE_BYTES,
        }
    }
}
#[derive(Default)]
struct State {
    epoch: u64,
    signed_in: bool,
    entries: VecDeque<Entry>,
    bytes: usize,
}
struct Entry {
    reference: String,
    connector: String,
    content: String,
    pending_file: bool,
}

impl ResultStore {
    fn lock_state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                // A panicked writer may have left the cache partly updated. Drop its
                // entries and expire old references, but keep the account signed in.
                let mut state = poisoned.into_inner();
                let epoch = state.epoch.wrapping_add(1);
                let signed_in = state.signed_in;
                *state = State {
                    epoch,
                    signed_in,
                    ..State::default()
                };
                self.state.clear_poison();
                state
            }
        }
    }

    pub fn activate(&self, signed_in: bool) {
        let mut state = self.lock_state();
        state.epoch += 1;
        state.signed_in = signed_in;
        state.entries.clear();
        state.bytes = 0;
    }
    pub fn is_active(&self) -> bool {
        self.lock_state().signed_in
    }
    pub fn scope(&self) -> u64 {
        self.lock_state().epoch
    }
    /// Only this explicitly requested path can create conversion receipts or bypass prose limits.
    pub fn preview_with_transport(
        &self,
        scope: u64,
        connector: &str,
        value: Value,
        allow_file: bool,
    ) -> Result<Value, ToolError> {
        let mut state = self.lock_state();
        check_scope(&state, scope)?;
        if allow_file
            && connector == "filesystem"
            && value.get("type").and_then(Value::as_str) == Some("cortex_file")
            && value
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.len() <= 8192)
            && value
                .get("data")
                .and_then(Value::as_str)
                .is_some_and(|data| data.len() <= MAX_INLINE_BASE64_LEN)
        {
            let transfer = register_file_transfer(&mut state, self.capacity, connector, value)?;
            return Ok(json!({"value":transfer, "file_transport":true}));
        }
        if connector.starts_with("mcp-")
            && value
                .get("content")
                .and_then(Value::as_array)
                .is_some_and(|blocks| blocks.iter().any(is_mcp_image))
        {
            let mut value = value;
            let mut files = Vec::new();
            let mut total_bytes = 0;
            let blocks = value
                .get_mut("content")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| {
                    ToolError::InvalidInput("MCP image result requires a content array.".into())
                })?;
            for (index, block) in blocks.iter_mut().enumerate() {
                if !is_mcp_image(block) {
                    continue;
                }
                let image = std::mem::replace(
                    block,
                    json!({"type":"text","text":format!("[Tool image {} provided separately]", index + 1)}),
                );
                let image = image.get("resource").unwrap_or(&image);
                let Some(data) = image
                    .get("data")
                    .or_else(|| image.get("blob"))
                    .and_then(Value::as_str)
                else {
                    *block =
                        json!({"type":"text","text":"[Tool image omitted: missing image data]"});
                    continue;
                };
                let mime = image.get("mimeType").and_then(Value::as_str).unwrap_or("");
                let label = image
                    .get("uri")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{connector} image {}", index + 1));
                block["text"] = json!(format!(
                    "[Tool image {} provided separately: {label}]",
                    index + 1
                ));
                let size = if data.len() <= MAX_INLINE_BASE64_LEN {
                    STANDARD.decode(data).ok().map(|bytes| bytes.len())
                } else {
                    None
                };
                let Some(size) = size.filter(|size| *size > 0 && *size <= MAX_INLINE_DATA_BYTES)
                else {
                    *block = json!({"type":"text","text":"[Tool image omitted: invalid data or image exceeds 5 MiB]"});
                    continue;
                };
                if !mime.starts_with("image/")
                    || files.len() >= 10
                    || total_bytes + size > 20 * 1024 * 1024
                {
                    *block = json!({"type":"text","text":"[Tool image omitted: unsupported MIME type or image output limit reached. Narrow a read-only query; do not repeat a write to recover output.]"});
                    continue;
                }
                total_bytes += size;
                files.push(register_file_transfer(
                    &mut state,
                    self.capacity,
                    connector,
                    json!({
                        "type":"cortex_file", "file_id":uuid::Uuid::new_v4().to_string(),
                        "path":label, "name":format!("tool-image-{}", index + 1),
                        "mime_type":mime, "size":size, "binary":true, "data":data,
                    }),
                )?);
            }
            let is_error = value
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            drop(state);
            let text = self.preview(scope, connector, value)?;
            return Ok(json!({"value":{
                "type":"cortex_tool_result","text":text,"files":files,"isError":is_error
            }, "file_transport":true}));
        }
        drop(state);
        let value = self.preview(scope, connector, value)?;
        Ok(json!({"value":value, "file_transport":false}))
    }

    pub fn preview(&self, scope: u64, connector: &str, value: Value) -> Result<Value, ToolError> {
        let mut state = self.lock_state();
        check_scope(&state, scope)?;
        let is_error = value
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let original = value.to_string();
        if original.len() <= MAX_PREVIEW_BYTES {
            return Ok(value);
        }
        let compact = if connector.starts_with("mcp-") {
            normalize_mcp(value.clone()).to_string()
        } else {
            original.clone()
        };
        if compact.len() <= MAX_PREVIEW_BYTES {
            return Ok(Value::String(compact));
        }
        let mut footer = String::new();
        let notice = if original.len() <= self.capacity {
            while state.bytes + original.len() > self.capacity {
                if let Some(old) = state.entries.pop_front() {
                    state.bytes -= old.content.len();
                }
            }
            let reference = uuid::Uuid::new_v4().to_string();
            footer = format!(
                "\n[Cortex saved tool result: result_id={reference}; reader=tool_results__read]"
            );
            state.bytes += original.len();
            state.entries.push_back(Entry {
                reference: reference.clone(),
                connector: connector.into(),
                content: original,
                pending_file: false,
            });
            format!("\n\n[Tool result truncated. Read omitted output with tool_results__read: result_id={reference} offset=0 limit=16000. Offsets are UTF-8 bytes. Do not repeat a write operation to recover its output. References expire on logout, restart or cache eviction.]\n\n")
        } else {
            "\n\n[Tool result truncated; too large to retain for paging. Use a narrower read-only source query. Do not repeat a write operation to recover its output.]\n\n".into()
        };
        let notice = if is_error {
            format!("{notice}[MCP isError=true: the tool reported an error.]\n")
        } else {
            notice
        };
        let available = MAX_PREVIEW_BYTES - notice.len() - footer.len();
        let tail = available / 4;
        let preview = format!(
            "{}{}{}{}",
            &compact[..compact.floor_char_boundary(available - tail)],
            notice,
            &compact[compact.ceil_char_boundary(compact.len() - tail)..],
            footer
        );
        Ok(Value::String(preview))
    }
    #[cfg(any(feature = "hosted", feature = "test-utils", test))]
    pub fn source_connector(&self, scope: u64, input: &Value) -> Result<String, ToolError> {
        let state = self.lock_state();
        check_scope(&state, scope)?;
        Ok(find_entry(&state, input)?.connector.clone())
    }
    /// Replace a one-use file receipt with converted output. The same preview,
    /// paging, account expiry and eviction rules apply as for any other result.
    pub fn complete_file(&self, scope: u64, input: &Value) -> Result<Value, ToolError> {
        let value = input
            .get("value")
            .ok_or_else(|| ToolError::InvalidInput("value is required".into()))?;
        if value.to_string().len() > 8 * 1024 * 1024
            || value.get("type").and_then(Value::as_str) == Some("cortex_file")
        {
            return Err(ToolError::InvalidInput(
                "Invalid converted file result.".into(),
            ));
        }
        let connector = {
            let mut state = self.lock_state();
            check_scope(&state, scope)?;
            let entry = find_entry(&state, input)?;
            if !entry.pending_file {
                return Err(ToolError::InvalidInput("Not a pending file read.".into()));
            }
            let reference = entry.reference.clone();
            let index = state
                .entries
                .iter()
                .position(|entry| entry.reference == reference)
                .ok_or_else(|| {
                    ToolError::ExecutionFailed("Pending file receipt is unavailable.".into())
                })?;
            let entry = state.entries.remove(index).ok_or_else(|| {
                ToolError::ExecutionFailed("Pending file receipt is unavailable.".into())
            })?;
            state.bytes -= entry.content.len();
            entry.connector
        };
        // Processed image bytes travel separately from bounded tool prose.
        if value.get("type").and_then(Value::as_str) == Some("cortex_file_image")
            && value.get("image_id").and_then(Value::as_str).is_some()
            && value
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.len() <= 8192)
            && value
                .get("mime_type")
                .and_then(Value::as_str)
                .is_some_and(|mime| {
                    matches!(
                        mime,
                        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp"
                    )
                })
            && value
                .get("data")
                .and_then(Value::as_str)
                .is_some_and(|data| data.len() <= MAX_INLINE_BASE64_LEN)
        {
            return Ok(value.clone());
        }
        self.preview(scope, &connector, value.clone())
    }
    pub fn read(&self, scope: u64, input: &Value) -> Result<Value, ToolError> {
        let state = self.lock_state();
        check_scope(&state, scope)?;
        let entry = find_entry(&state, input)?;
        if entry.pending_file {
            return Err(ToolError::InvalidInput(
                "File conversion has not completed.".into(),
            ));
        }
        let offset = integer(input, "offset", 0)?;
        let limit = integer(input, "limit", MAX_PAGE_BYTES)?.min(MAX_PAGE_BYTES);
        if limit <= 512 {
            return Err(ToolError::InvalidInput(
                "limit must be greater than 512 bytes (including the page header)".into(),
            ));
        }
        if offset > entry.content.len() || !entry.content.is_char_boundary(offset) {
            return Err(ToolError::InvalidInput(
                "offset must be a UTF-8 character boundary within the saved output".into(),
            ));
        }
        let end = entry
            .content
            .floor_char_boundary(offset.saturating_add(limit - 512).min(entry.content.len()));
        if end == offset && offset < entry.content.len() {
            return Err(ToolError::InvalidInput(
                "limit is too small to include the next UTF-8 character and page header".into(),
            ));
        }
        let next = (end < entry.content.len()).then_some(end);
        let header = json!({"result_id":entry.reference,"offset":offset,"next_offset":next,"total_bytes":entry.content.len()});
        Ok(Value::String(format!(
            "{header}\n{}",
            &entry.content[offset..end]
        )))
    }
}

fn is_mcp_image(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("image")
        || (block.get("type").and_then(Value::as_str) == Some("resource")
            && block.get("resource").is_some_and(|resource| {
                resource.get("blob").is_some()
                    && resource
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .is_some_and(|mime| mime.starts_with("image/"))
            }))
}

fn register_file_transfer(
    state: &mut State,
    capacity: usize,
    connector: &str,
    value: Value,
) -> Result<Value, ToolError> {
    // Retain only an authorization receipt, not a second copy of file bytes.
    let reference = uuid::Uuid::new_v4().to_string();
    let content = "[File conversion pending]".to_string();
    while state.bytes + content.len() > capacity {
        if let Some(old) = state.entries.pop_front() {
            state.bytes -= old.content.len();
        } else {
            return Err(ToolError::ExecutionFailed(
                "Tool result cache is full.".into(),
            ));
        }
    }
    state.bytes += content.len();
    state.entries.push_back(Entry {
        reference: reference.clone(),
        connector: connector.into(),
        content,
        pending_file: true,
    });
    let mut value = value;
    value["result_id"] = json!(reference);
    Ok(value)
}

fn check_scope(state: &State, scope: u64) -> Result<(), ToolError> {
    if !state.signed_in || state.epoch != scope {
        return Err(ToolError::InvalidInput("Tool result belongs to an expired account session. Do not repeat a write to recover its output.".into()));
    }
    Ok(())
}
fn integer(input: &Value, key: &str, default: usize) -> Result<usize, ToolError> {
    match input.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| ToolError::InvalidInput(format!("{key} must be a nonnegative integer"))),
    }
}
fn find_entry<'a>(state: &'a State, input: &Value) -> Result<&'a Entry, ToolError> {
    let id = input
        .get("result_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidInput("result_id is required".into()))?;
    state.entries.iter().find(|entry| entry.reference == id).ok_or_else(|| ToolError::InvalidInput("Saved tool result expired or is unavailable. Use a narrower read-only source query; do not repeat a write operation.".into()))
}

// Some MCP servers repeat identical JSON in both text and structuredContent.
// Remove that duplication only after checking equivalence; preserve other blocks and metadata.
fn normalize_mcp(mut value: Value) -> Value {
    if let Some(blocks) = value.get_mut("content").and_then(Value::as_array_mut) {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str).map(str::to_owned) {
                    block["text"] = Value::String(minify_json(&text));
                }
            }
        }
    }
    let duplicate = value
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.len() == 1
                && blocks[0].get("type").and_then(Value::as_str) == Some("text")
                && value.get("structuredContent").is_some_and(|data| {
                    blocks[0].get("text").and_then(Value::as_str) == Some(data.to_string().as_str())
                })
        });
    if duplicate {
        if let Some(object) = value.as_object_mut() {
            object.remove("structuredContent");
        }
    }
    value
}

fn minify_json(text: &str) -> String {
    if serde_json::from_str::<Value>(text).is_err() {
        return text.into();
    }
    // Keep the original numeric literals, duplicate keys and string contents.
    let mut quoted = false;
    let mut escaped = false;
    text.chars()
        .filter(|&ch| {
            if quoted {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quoted = false;
                }
                true
            } else if ch == '"' {
                quoted = true;
                true
            } else {
                !matches!(ch, ' ' | '\t' | '\r' | '\n')
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
