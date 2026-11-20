//! A finished run posted to the task's webhook, in a shape Slack and Discord read too.

use super::*;

/// What a webhook receives when a run finishes.
pub(in crate::chat::automation) fn webhook_payload(
    task: &Task,
    conversation_id: &str,
    status: TaskStatus,
    answer: &str,
) -> serde_json::Value {
    let answer: String = crate::support::text::clip(answer, 20_000);
    serde_json::json!({
        "event": "task.finished",
        "task": { "id": task.id, "title": task.title },
        "status": status.as_str(),
        "conversationId": conversation_id,
        "answer": answer,
        // Slack and Discord incoming webhooks read these fields.
        "text": format!("{}: {}", task.title, crate::support::text::clip(&answer, 3_000)),
        "content": format!("**{}**\n{}", task.title, crate::support::text::clip(&answer, 1_900)),
        "finishedAt": chrono::Utc::now(),
    })
}

/// Posts a finished run to the task's webhook, when it has one (http or https only).
pub async fn notify_webhook(task: &Task, conversation_id: &str, status: TaskStatus, answer: &str) {
    let Some(url) = task
        .webhook
        .as_deref()
        .map(str::trim)
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
    else {
        return;
    };
    let Ok(client) = crate::support::http::client(std::time::Duration::from_secs(15)) else {
        return;
    };
    match client
        .post(url)
        .json(&webhook_payload(task, conversation_id, status, answer))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => log::warn!("the webhook of {} answered {}", task.title, r.status()),
        Err(e) => log::warn!("the webhook of {} failed: {e}", task.title),
    }
}
