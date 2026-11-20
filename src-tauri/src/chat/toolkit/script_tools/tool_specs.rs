//! The schemas of the tools that write, promote, retire and run tools.

use super::*;

/// The definitions of `create_tool` and `promote_tool`.
pub(in crate::chat) fn builtin_definitions() -> Vec<ChatTool> {
    let tool = |name: &str, description: &str, parameters: Value, read_only: bool| ChatTool {
        name: name.into(),
        description: description.into(),
        parameters,
        connector_id: None,
        tool_name: name.into(),
        read_only,
    };
    vec![
        tool(
            RUN_SCRIPT,
            "Run a short JavaScript program once, in a sandbox with no network, files or other \
             tools: for an exact calculation, parsing or reformatting. Define `function main(args)` \
             and return a JSON value (or print with console.log). Nothing is saved; for something \
             you will repeat, write a tool with create_tool.",
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "JavaScript defining function main(args), or a program that prints." },
                    "args": { "type": "object", "description": "The arguments main receives." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "memory_mb": { "type": "integer", "minimum": 16, "maximum": 128 }
                },
                "required": ["code"]
            }),
            true,
        ),
        tool(
            CREATE_TOOL,
            "Write a reusable tool for yourself when a task needs a small, exact computation or \
             fetch you would otherwise repeat: a JavaScript script that defines `function \
             main(args)` and returns a JSON value. It runs in a sandbox with no network or files \
             unless you ask for them: `fetch(method, url, body)` (https, listed hosts only) returns \
             {status, body}; `read(path)` reads a file in the chat's folder and `write(path, text)` \
             writes one where allowed; `log(text)` notes progress. It is saved as a candidate and tested once on `test_args`; call \
             promote_tool once it works, and the user decides. `tool(name, args)` calls another tool \
             you wrote, when you list it in `tools`.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "lowercase_with_underscores" },
                    "description": { "type": "string", "description": "What it does and when to use it, for your future self." },
                    "parameters": { "type": "object", "description": "JSON schema of the arguments." },
                    "script": { "type": "string", "description": "JavaScript defining function main(args)." },
                    "test_args": { "type": "object", "description": "Arguments for the test run." },
                    "http_hosts": { "type": "array", "items": { "type": "string" }, "description": "Exact host names it needs to reach over https, if any." },
                    "read_prefixes": { "type": "array", "items": { "type": "string" }, "description": "Folders of the chat's workspace it needs to read, if any." },
                    "write_prefixes": { "type": "array", "items": { "type": "string" }, "description": "Folders of the chat's workspace it needs to write, if any; `write(path, text)` writes there." },
                    "credentials": { "type": "array", "items": { "type": "string" }, "description": "Vault secrets its requests need, as host=SECRET_NAME; the app sends them as a bearer token to that host. The script never sees them." },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Other tools you wrote that it calls with tool(name, args), so their results never pass through the chat." },
                    "new_anyway": { "type": "boolean", "description": "Save it although a similar tool exists (you were told which); prefer updating that one." }
                },
                "required": ["name", "description", "script"]
            }),
            false,
        ),
        tool(
            PROMOTE_TOOL,
            "Ask the user to make a tool you wrote with create_tool available in every chat. They \
             see its script, description and access, and approve or decline.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "days": { "type": "integer", "minimum": 1, "description": "Grant its network and file access for this many days only." }
                },
                "required": ["name"]
            }),
            false,
        ),
        tool(
            RETIRE_TOOL,
            "Retire a tool you wrote that is no longer useful or was replaced; it stops being \
             offered and can be promoted again later.",
            json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
            false,
        ),
        tool(
            REQUEST_CAPABILITY,
            "Ask the user to let one of your tools reach more: https hosts or folders of the chat's \
             workspace, for a number of days, with the reason. Nothing changes until they approve.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "http_hosts": { "type": "array", "items": { "type": "string" } },
                    "read_prefixes": { "type": "array", "items": { "type": "string" } },
                    "write_prefixes": { "type": "array", "items": { "type": "string" } },
                    "credentials": { "type": "array", "items": { "type": "string" } },
                    "tools": { "type": "array", "items": { "type": "string" } },
                    "days": { "type": "integer", "minimum": 1 },
                    "reason": { "type": "string" }
                },
                "required": ["name", "reason"]
            }),
            false,
        ),
    ]
}
