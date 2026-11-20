//! Handlers for working with others and with the chat: delegating to workers, messaging and managing agents, finding tools, the notebook and canvases.

use super::*;

#[allow(unused_variables, unused_mut)]
pub(super) fn on_delegate<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        match subagents::parse_tasks(&arguments) {
            Ok(tasks) => {
                let reports = subagents::run_all(
                    r.host,
                    r.config_dir,
                    &r.request.model,
                    r.offered,
                    r.id,
                    &call.id.to_string(),
                    tasks,
                    subagents::Mode::parse(&arguments),
                )
                .await;
                tools::Outcome {
                    text: subagents::render(&reports),
                    is_error: reports
                        .iter()
                        .all(|r| r.status != crate::chat::events::RunStatus::Done),
                }
            }
            Err(e) => tools::Outcome::err(e),
        }
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_message_agent<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return message_agent(
            r.host,
            r.config_dir,
            r.id,
            r.agent,
            &arguments,
            r.workdir.as_deref(),
            r.request.approval,
        )
        .await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_manage_agents<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return r.manage_agents(&arguments, asked).await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_find_tools<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let semantic = semantic_tool_ranking(
            r.host.chat_state().tool_vectors(),
            r.config_dir,
            r.offered,
            query,
        )
        .await;
        tools::Outcome::ok(deferred::find_with(r.offered, &arguments, semantic).0)
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_notebook<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let Some(agent_id) = store::load_conversation(r.config_dir, r.id)
            .ok()
            .and_then(|c| c.agent_id)
        else {
            return tools::Outcome::err("Only agents keep a notebook.");
        };
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match agents::save_notebook(r.config_dir, &agent_id, content) {
            Ok(()) => tools::Outcome::ok("Notebook saved."),
            Err(e) => tools::Outcome::err(e),
        }
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_canvas<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let text = |k: &str| arguments.get(k).and_then(Value::as_str).map(str::to_string);
        // A bigger app comes as several files.
        let files: Option<Vec<canvas::AppFile>> = arguments
            .get("files")
            .and_then(|f| serde_json::from_value(f.clone()).ok())
            .filter(|f: &Vec<canvas::AppFile>| !f.is_empty());
        let content = match files {
            Some(files) => match canvas::bundle(&files) {
                Ok(c) => Some(c),
                Err(e) => return tools::Outcome::err(e),
            },
            None => text("content").filter(|c| !c.trim().is_empty()),
        };
        let Some(content) = content else {
            return tools::Outcome::err("Give the canvas content.");
        };
        let id = text("id").filter(|i| !i.trim().is_empty());
        let kind = if arguments.get("files").is_some() {
            Some("app".to_string())
        } else {
            text("kind")
        };
        match canvas::write_kind(r.config_dir, r.id, id.as_deref(), text("title").as_deref(), &content, "assistant", kind.as_deref()) {
            Ok(c) => tools::Outcome::ok(serde_json::json!({ "id": c.id, "title": c.title, "kind": c.kind, "version": c.versions.len() }).to_string()),
            Err(e) => tools::Outcome::err(e),
        }
    })
}
