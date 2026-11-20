//! OpenAPI tool servers, as Open WebUI offers: point the app at a service's `openapi.json` and
//! each operation becomes a tool the assistant can call.
//!
//! The spec is fetched when the server is added (or refreshed) and kept under
//! `chat/openapi/<id>.json`, so a turn never waits on the network to know its tools. Path and
//! query parameters become the tool's arguments, a JSON request body goes under `body`. GET and
//! HEAD operations count as reads; the rest follow the approval mode like any connector tool.
//! A key, when given, is kept in the keychain and sent as a bearer token.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::store::{chat_dir, write_json};
use super::tools::ChatTool;

/// Operations offered per server.
const MAX_OPERATIONS: usize = 60;
/// Characters of a response handed to the model.
const MAX_RESPONSE: usize = 20_000;
pub const PREFIX: &str = "openapi-";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub spec_url: String,
    /// Where calls go: the spec's first server, resolved against the spec's address.
    pub base_url: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub has_key: bool,
    /// How many operations the spec offers (for the settings list).
    #[serde(default)]
    pub operations: usize,
}

fn yes() -> bool {
    true
}

fn dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("openapi")
}

pub fn list(config_dir: &Path) -> Vec<Server> {
    crate::chat::at_rest::read_to_string(dir(config_dir).join("servers.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn store(config_dir: &Path, servers: &[Server]) -> Result<(), String> {
    write_json(&dir(config_dir).join("servers.json"), &servers)
}

fn spec_path(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || id.is_empty() {
        return Err("invalid server id".into());
    }
    Ok(dir(config_dir).join(format!("{id}.json")))
}

pub fn secret_name(id: &str) -> String {
    format!("openapi-{id}")
}

/// A short, tool-name-safe id from the server's name.
fn slug(name: &str) -> String {
    crate::support::names::slug(name, 24)
}

/// The spec's base URL: its first `servers` entry, resolved against where the spec came from.
pub fn base_url(spec: &Value, spec_url: &str) -> Result<String, String> {
    let declared = spec
        .pointer("/servers/0/url")
        .and_then(Value::as_str)
        .unwrap_or("/");
    let base = reqwest::Url::parse(spec_url).map_err(|e| format!("{spec_url}: {e}"))?;
    let joined = base.join(declared).map_err(|e| e.to_string())?;
    Ok(joined.as_str().trim_end_matches('/').to_string())
}

/// Resolves local `$ref`s (`#/components/schemas/X`), a few levels deep.
fn resolve(spec: &Value, schema: &Value, depth: usize) -> Value {
    if depth > 6 {
        return json!({});
    }
    match schema {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                let target = reference
                    .strip_prefix('#')
                    .and_then(|p| spec.pointer(p))
                    .cloned()
                    .unwrap_or(json!({}));
                return resolve(spec, &target, depth + 1);
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), resolve(spec, v, depth + 1)))
                    .collect(),
            )
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| resolve(spec, v, depth + 1)).collect())
        }
        other => other.clone(),
    }
}

/// One operation of a spec, ready to be a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub id: String,
    pub method: String,
    pub path: String,
    pub description: String,
    pub parameters: Value,
}

/// The operations a spec offers, with their argument schemas.
pub fn operations(spec: &Value) -> Vec<Operation> {
    let mut out = Vec::new();
    let Some(paths) = spec.get("paths").and_then(Value::as_object) else {
        return out;
    };
    for (path, item) in paths {
        let shared: Vec<Value> = item
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(op) = item.get(method) else { continue };
            let id = op
                .get("operationId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    let words: Vec<&str> = path
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .filter(|w| !w.is_empty())
                        .collect();
                    format!("{method}_{}", words.join("_"))
                });
            let mut properties = Map::new();
            let mut required: Vec<Value> = Vec::new();
            let params = shared.iter().chain(
                op.get("parameters")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            );
            for p in params {
                let p = resolve(spec, p, 0);
                let (Some(name), Some(place)) = (
                    p.get("name").and_then(Value::as_str),
                    p.get("in").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if place != "path" && place != "query" {
                    continue;
                }
                let mut schema = p
                    .get("schema")
                    .cloned()
                    .unwrap_or(json!({ "type": "string" }));
                if let (Some(obj), Some(d)) = (
                    schema.as_object_mut(),
                    p.get("description").and_then(Value::as_str),
                ) {
                    obj.entry("description").or_insert(json!(d));
                }
                properties.insert(name.to_string(), schema);
                if place == "path" || p.get("required").and_then(Value::as_bool) == Some(true) {
                    required.push(json!(name));
                }
            }
            if let Some(body) = op.pointer("/requestBody/content/application~1json/schema") {
                properties.insert("body".into(), resolve(spec, body, 0));
                if op.pointer("/requestBody/required").and_then(Value::as_bool) == Some(true) {
                    required.push(json!("body"));
                }
            }
            let summary = op
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let detail = op
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let description: String = [summary, detail]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(". ")
                .chars()
                .take(500)
                .collect();
            out.push(Operation {
                id,
                method: method.to_uppercase(),
                path: path.clone(),
                description: if description.is_empty() { format!("{} {path}", method.to_uppercase()) } else { description },
                parameters: json!({ "type": "object", "properties": properties, "required": required }),
            });
            if out.len() >= MAX_OPERATIONS {
                return out;
            }
        }
    }
    out
}

/// The tools of every enabled server.
pub fn tools(config_dir: &Path) -> Vec<ChatTool> {
    let mut out = Vec::new();
    for server in list(config_dir).into_iter().filter(|s| s.enabled) {
        let Ok(path) = spec_path(config_dir, &server.id) else {
            continue;
        };
        let Some(spec) = crate::chat::at_rest::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        for op in operations(&spec) {
            let read_only = op.method == "GET";
            out.push(ChatTool {
                name: super::tools::safe_name(&format!("api-{}__{}", server.id, op.id)),
                description: format!("{} (via {})", op.description, server.name),
                parameters: op.parameters,
                connector_id: Some(format!("{PREFIX}{}", server.id)),
                tool_name: op.id,
                read_only,
            });
        }
    }
    out
}

async fn fetch_spec(spec_url: &str) -> Result<Value, String> {
    let client = crate::support::http::client(std::time::Duration::from_secs(30))?;
    let response = client
        .get(spec_url)
        .send()
        .await
        .map_err(|e| format!("Could not fetch the spec: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("The spec address answered {}.", response.status()));
    }
    let text = response.text().await.map_err(|e| e.to_string())?;
    let spec: Value = serde_json::from_str(&text)
        .map_err(|_| "The address does not return an OpenAPI document in JSON.".to_string())?;
    if spec.get("openapi").is_none() && spec.get("swagger").is_none() {
        return Err("The document is not an OpenAPI spec (no `openapi` field).".into());
    }
    if operations(&spec).is_empty() {
        return Err("The spec has no operations.".into());
    }
    Ok(spec)
}

/// Adds a server from its spec address; returns the list.
pub async fn add(config_dir: &Path, name: &str, spec_url: &str) -> Result<Vec<Server>, String> {
    let spec_url = spec_url.trim();
    let spec = fetch_spec(spec_url).await?;
    let title = spec
        .pointer("/info/title")
        .and_then(Value::as_str)
        .unwrap_or("API");
    let name = if name.trim().is_empty() {
        title.to_string()
    } else {
        name.trim().to_string()
    };
    let mut servers = list(config_dir);
    let mut id = slug(&name);
    if id.is_empty() {
        id = "api".into();
    }
    let base = id.clone();
    let mut n = 2;
    while servers.iter().any(|s| s.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    std::fs::create_dir_all(dir(config_dir)).map_err(|e| e.to_string())?;
    write_json(&spec_path(config_dir, &id)?, &spec)?;
    servers.push(Server {
        base_url: base_url(&spec, spec_url)?,
        operations: operations(&spec).len(),
        id,
        name,
        spec_url: spec_url.into(),
        enabled: true,
        has_key: false,
    });
    store(config_dir, &servers)?;
    Ok(servers)
}

pub async fn refresh(config_dir: &Path, id: &str) -> Result<Vec<Server>, String> {
    let mut servers = list(config_dir);
    let server = servers
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or("No such server.")?;
    let spec = fetch_spec(&server.spec_url).await?;
    server.base_url = base_url(&spec, &server.spec_url)?;
    server.operations = operations(&spec).len();
    write_json(&spec_path(config_dir, id)?, &spec)?;
    store(config_dir, &servers)?;
    Ok(servers)
}

pub fn update(
    config_dir: &Path,
    id: &str,
    enabled: Option<bool>,
    has_key: Option<bool>,
) -> Result<Vec<Server>, String> {
    let mut servers = list(config_dir);
    let server = servers
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or("No such server.")?;
    if let Some(e) = enabled {
        server.enabled = e;
    }
    if let Some(k) = has_key {
        server.has_key = k;
    }
    store(config_dir, &servers)?;
    Ok(servers)
}

pub fn remove(config_dir: &Path, id: &str) -> Result<Vec<Server>, String> {
    let mut servers = list(config_dir);
    servers.retain(|s| s.id != id);
    let _ = std::fs::remove_file(spec_path(config_dir, id)?);
    store(config_dir, &servers)?;
    Ok(servers)
}

/// The request an operation call makes: method, full URL and JSON body.
pub fn request_for(
    base_url: &str,
    op: &Operation,
    input: &Value,
) -> Result<(String, String, Option<Value>), String> {
    let args = input.as_object().cloned().unwrap_or_default();
    let mut path = op.path.clone();
    let mut query: Vec<(String, String)> = Vec::new();
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let props = op
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (name, value) in &args {
        if name == "body" || !props.contains_key(name) {
            continue;
        }
        let placeholder = format!("{{{name}}}");
        if path.contains(&placeholder) {
            let encoded: String = url_escape(&text(value));
            path = path.replace(&placeholder, &encoded);
        } else {
            query.push((name.clone(), text(value)));
        }
    }
    if path.contains('{') {
        return Err(format!("Missing a path value for {path}."));
    }
    let mut url = reqwest::Url::parse(&format!("{base_url}{path}")).map_err(|e| e.to_string())?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query.iter());
    }
    Ok((
        op.method.clone(),
        url.to_string(),
        args.get("body").cloned(),
    ))
}

fn url_escape(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Calls an operation of a server.
pub async fn call(config_dir: &Path, tool: &ChatTool, input: &Value) -> Result<String, String> {
    let id = tool
        .connector_id
        .as_deref()
        .and_then(|c| c.strip_prefix(PREFIX))
        .ok_or("not an OpenAPI tool")?;
    let server = list(config_dir)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("That API server was removed.")?;
    let spec: Value = crate::chat::at_rest::read_to_string(spec_path(config_dir, id)?)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or("The API's spec is missing; refresh it in Settings.")?;
    let op = operations(&spec)
        .into_iter()
        .find(|o| o.id == tool.tool_name)
        .ok_or("That operation is no longer in the spec.")?;
    let (method, url, body) = request_for(&server.base_url, &op, input)?;
    let client = crate::support::http::client(std::time::Duration::from_secs(60))?;
    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?,
        &url,
    );
    if server.has_key {
        let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
        if let Some(key) = crate::tools::auth::keychain_secrets(account)
            .load(&secret_name(id))
            .await?
        {
            request = request.bearer_auth(key);
        }
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let text: String = text.chars().take(MAX_RESPONSE).collect();
    if status.is_success() {
        Ok(if text.is_empty() {
            format!("{} ({})", status, method)
        } else {
            text
        })
    } else {
        Err(format!("{method} {url} answered {status}: {text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Value {
        json!({
            "openapi": "3.0.0",
            "info": { "title": "Weather & Notes" },
            "servers": [{ "url": "/api" }],
            "paths": {
                "/weather/{city}": {
                    "get": {
                        "operationId": "getWeather",
                        "summary": "Current weather",
                        "parameters": [
                            { "name": "city", "in": "path", "required": true, "schema": { "type": "string" } },
                            { "name": "units", "in": "query", "schema": { "type": "string", "enum": ["c", "f"] }, "description": "Units" },
                            { "name": "X-Trace", "in": "header", "schema": { "type": "string" } }
                        ]
                    }
                },
                "/notes": {
                    "post": {
                        "summary": "Add a note",
                        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Note" } } } }
                    }
                }
            },
            "components": { "schemas": { "Note": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] } } }
        })
    }

    #[test]
    fn operations_become_tool_schemas() {
        let ops = operations(&spec());
        assert_eq!(ops.len(), 2);
        let weather = ops.iter().find(|o| o.id == "getWeather").unwrap();
        assert_eq!(weather.method, "GET");
        assert_eq!(weather.parameters["required"], json!(["city"]));
        assert_eq!(
            weather.parameters["properties"]["units"]["description"],
            "Units"
        );
        assert!(
            weather.parameters["properties"].get("X-Trace").is_none(),
            "headers are not arguments"
        );
        let note = ops.iter().find(|o| o.method == "POST").unwrap();
        assert_eq!(note.id, "post_notes");
        assert_eq!(
            note.parameters["properties"]["body"]["properties"]["text"]["type"], "string",
            "$ref resolved"
        );
        assert_eq!(note.parameters["required"], json!(["body"]));
    }

    #[test]
    fn calls_fill_the_path_and_query() {
        let ops = operations(&spec());
        let base = base_url(&spec(), "http://127.0.0.1:9000/openapi.json").unwrap();
        assert_eq!(base, "http://127.0.0.1:9000/api");
        let weather = ops.iter().find(|o| o.id == "getWeather").unwrap();
        let (method, url, body) = request_for(
            &base,
            weather,
            &json!({ "city": "São Paulo", "units": "c", "stray": 1 }),
        )
        .unwrap();
        assert_eq!(
            (method.as_str(), url.as_str(), body),
            (
                "GET",
                "http://127.0.0.1:9000/api/weather/S%C3%A3o%20Paulo?units=c",
                None
            )
        );
        assert!(
            request_for(&base, weather, &json!({})).is_err(),
            "a path value is required"
        );
        let note = ops.iter().find(|o| o.method == "POST").unwrap();
        let (_, url, body) =
            request_for(&base, note, &json!({ "body": { "text": "hi" } })).unwrap();
        assert_eq!(url, "http://127.0.0.1:9000/api/notes");
        assert_eq!(body, Some(json!({ "text": "hi" })));
        assert_eq!(slug("Weather & Notes!"), "weather-notes");
    }
}
