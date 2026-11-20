//! Handlers for running code and commands, the browser, and making images, animations and video.

use super::*;

#[allow(unused_variables, unused_mut)]
pub(super) fn on_run_code<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return r.run_code(&arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_terminal<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return r.terminal(name, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_browser_act<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        // Using a page can change things (submit, sign up, post), so it asks unless the
        // chat runs on its own by choice.
        if !asked
            && !matches!(
                r.request.approval,
                ApprovalMode::Auto | ApprovalMode::Autonomous
            )
        {
            if r.request.approval.unattended() {
                return tools::Outcome::err(
                    "Using web pages needs the user's approval, and nobody is watching this run.",
                );
            }
            let gate = ChatTool {
                name: name.into(),
                description: String::new(),
                parameters: Value::Null,
                connector_id: Some("browser".into()),
                tool_name: name.into(),
                read_only: false,
            };
            if matches!(
                r.host.approve(&gate, &arguments, None).await,
                ApprovalDecision::Deny
            ) {
                return tools::Outcome {
                    text:
                        "The user declined this. Do not retry it; ask what they would like instead."
                            .into(),
                    is_error: true,
                };
            }
        }
        return crate::chat::webpage::act(
            r.config_dir,
            r.id,
            r.agent.map(|a| a.id.as_str()),
            &arguments,
        )
        .await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_browser_view<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return crate::chat::webpage::view(
            r.config_dir,
            r.id,
            r.agent.map(|a| a.id.as_str()),
            &arguments,
        )
        .await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_generate_image<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return media::generate(r.config_dir, r.id, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_make_animation<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return crate::chat::animation::make(r.config_dir, r.id, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
pub(super) fn on_generate_video<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return crate::chat::videogen::generate(r.config_dir, r.id, &arguments).await;
    })
}
