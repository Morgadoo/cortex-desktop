//! A scheduled task's run: its chat, model and tools, and what is recorded and notified.

use super::*;

/// Runs a scheduled task: a new unread conversation with the task's prompt, under its agent.
pub(in crate::chat) async fn run_task(app: AppHandle, task: tasks::Task) {
    let Ok(config_dir) = config_dir(&app) else {
        return;
    };
    let agent = task
        .agent_id
        .as_deref()
        .and_then(|a| agents::find(&config_dir, a));
    let providers_file = providers::load(&config_dir);
    let model = task
        .model
        .clone()
        .or_else(|| agent.as_ref().and_then(|a| a.model.clone()))
        .or(providers_file.default_model.clone());
    let Some(model) = model else {
        warn!("scheduled task {} has no model to run on", task.title);
        tasks::record(&config_dir, &task.id, "", tasks::TaskStatus::Failed);
        return;
    };
    // A follow-up continues the chat it was asked in, while that chat still exists.
    let follow_up = task
        .reply_in
        .as_ref()
        .and_then(|id| store::load_conversation(&config_dir, id).ok())
        .filter(|c| !c.archived);
    let id = match follow_up {
        Some(mut conversation) => {
            conversation.unread = true;
            if let Err(e) = store::save_conversation(&config_dir, &conversation) {
                warn!("could not continue a chat for {}: {e}", task.title);
                return;
            }
            conversation.id
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let mut conversation = Conversation::new(id.clone(), None, Some(model.clone()));
            conversation.title = task.title.clone();
            conversation.agent_id = task.agent_id.clone();
            conversation.task_id = Some(task.id.clone());
            conversation.unread = true;
            if let Err(e) = store::save_conversation(&config_dir, &conversation) {
                warn!("could not start scheduled task {}: {e}", task.title);
                return;
            }
            id
        }
    };
    let prompt = tasks::run_prompt(&task);
    let text = if task.reply_in.as_deref() == Some(id.as_str()) {
        format!("(Scheduled follow-up: {}) {}", task.title, prompt)
    } else {
        prompt
    };
    let Some(cancel) = app.state::<ChatState>().begin(&id) else {
        return;
    };
    let request = TurnRequest {
        conversation_id: id.clone(),
        text,
        model,
        speed: if agent.as_ref().is_some_and(|a| a.think) {
            Speed::Think
        } else {
            Speed::Fast
        },
        approval: if task.allow_changes {
            ApprovalMode::Autonomous
        } else {
            ApprovalMode::Unattended
        },
        project_id: None,
        agent_id: task.agent_id.clone(),
        images: Vec::new(),
        edit_index: None,
        temporary: false,
        resume: false,
        workdir: None,
        connectors: None,
        route_note: None,
        tools_off: Vec::new(),
        max_steps: None,
        only_tools: None,
        system: None,
        skills_off: Vec::new(),
        facts_off: Vec::new(),
        pages_off: Vec::new(),
        instructions_off: false,
    };
    run_turn(app.clone(), request, cancel).await;
    let finished = store::load_conversation(&config_dir, &id).ok();
    let answered = finished
        .as_ref()
        .and_then(|c| c.messages.last())
        .is_some_and(|m| matches!(m, Message::Assistant(_)));
    tasks::record(
        &config_dir,
        &task.id,
        &id,
        if answered {
            tasks::TaskStatus::Ok
        } else {
            tasks::TaskStatus::Failed
        },
    );
    let answer = finished
        .as_ref()
        .and_then(|c| c.messages.last())
        .map(crate::chat::context::message_text)
        .unwrap_or_default();
    let status = if answered {
        tasks::TaskStatus::Ok
    } else {
        tasks::TaskStatus::Failed
    };
    tasks::notify_webhook(&task, &id, status, &answer).await;
    notify(
        &app,
        &task.title,
        if answered {
            "Your scheduled task finished."
        } else {
            "Your scheduled task could not finish."
        },
    );
}
