//! The built-ins that read the assistant's own record: run traces, usage and failures, and a past chat.

use super::*;

/// `QUERY_TRACES`.
pub(super) async fn query_traces(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let days = args
        .get("days")
        .and_then(Value::as_i64)
        .unwrap_or(7)
        .clamp(1, 365);
    let filter = crate::chat::traces::RunFilter {
        since: Some(chrono::Utc::now() - chrono::Duration::days(days)),
        outcome: Some(str_arg("outcome")).filter(|o| !o.is_empty()),
        tool: Some(str_arg("tool")).filter(|t| !t.is_empty()),
        failed_tools: args
            .get("failed_tools")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limit: args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
    };
    ok(
        serde_json::to_string_pretty(&crate::chat::traces::runs(config_dir, &filter))
            .unwrap_or_default(),
    )
}

/// `GET_RUN`.
pub(super) async fn get_run(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    match crate::chat::traces::run_detail(config_dir, &str_arg("id")) {
        Some(detail) => ok(serde_json::to_string_pretty(&detail).unwrap_or_default()),
        None => err("There is no run with that id; list them with query_traces."),
    }
}

/// `USAGE_STATS`.
pub(super) async fn usage_stats_of_artifact(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    // One artifact: how often it was in front of the model and used, and its score.
    let wanted = str_arg("artifact");
    let Some((kind, id)) = wanted.split_once(':') else {
        return err("Name the artifact as kind:id, e.g. skill:summarize or tool:web_search.");
    };
    match crate::chat::traces::artifact(config_dir, kind, id) {
        Some(a) => ok(serde_json::to_string_pretty(&json!({
            "artifact": wanted,
            "status": a.status,
            "version": a.version,
            "loaded": a.loaded,
            "used": a.used,
            "lastUsed": a.last_used,
            "critical": a.critical,
            "score": cortex_kernel::lifecycle::score(&a, chrono::Utc::now()),
        }))
        .unwrap_or_default()),
        None => err("Nothing by that name has been recorded."),
    }
}

/// `USAGE_STATS | FAILURE_SUMMARY`.
pub(super) async fn usage_or_failures(config_dir: &Path, args: &Value, name: &str) -> Outcome {
    let days = args.get("days").and_then(Value::as_i64).unwrap_or(7);
    ok(if name == USAGE_STATS {
        crate::chat::traces::usage_report(config_dir, days)
    } else {
        crate::chat::traces::failure_report(config_dir, days)
    })
}

/// `READ_CHAT`.
pub(super) async fn read_chat(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    match crate::chat::store::load_conversation(config_dir, &str_arg("id")) {
        Ok(c) if !c.temporary => ok(crate::chat::store::chat_transcript(&c, READ_CHAT_LIMIT)),
        _ => err("There is no conversation with that id; use an id from search_chats."),
    }
}
