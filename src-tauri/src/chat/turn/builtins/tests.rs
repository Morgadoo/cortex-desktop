use super::*;

#[test]
fn built_ins_that_change_something_are_not_offered_as_read_only() {
    let config = tempfile::tempdir().unwrap();
    let offered = definitions(config.path());
    let read_only = |name: &str| {
        offered
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is offered"))
            .read_only
    };
    for name in [SCHEDULE_TASK, SAVE_SKILL, REMEMBER, FORGET, KNOWLEDGE_WRITE] {
        assert!(!read_only(name), "{name} changes something");
    }
    for name in [WEB_SEARCH, USE_SKILL, KNOWLEDGE_SEARCH, SEARCH_CHATS] {
        assert!(read_only(name), "{name} only reads");
    }
    assert_eq!(effect(SCHEDULE_TASK), Effect::Standing);
    assert_eq!(effect(SAVE_SKILL), Effect::Standing);
    assert_eq!(effect(REMEMBER), Effect::Memory);
    assert_eq!(effect(READ_CHAT), Effect::Reads);
}

#[tokio::test]
async fn scheduling_from_chat_creates_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), SCHEDULE_TASK, &json!({"title": "AI news", "prompt": "Brief me", "frequency": "weekly", "time": "09:00", "days": ["monday"]})).await;
    assert!(!out.is_error, "{}", out.text);
    assert!(out.text.contains("Every Monday at 09:00"));
    assert_eq!(crate::chat::tasks::load(dir.path()).len(), 1);
    let bad = run(
        dir.path(),
        SCHEDULE_TASK,
        &json!({"title": "x", "prompt": "y", "frequency": "hourly", "time": "09:00"}),
    )
    .await;
    assert!(bad.is_error);
    // A follow-up in the chat it was asked in; `reply_in` is set by the engine, and only used
    // when the model asked for it.
    run(dir.path(), SCHEDULE_TASK, &json!({"title": "Check prices", "prompt": "Check again", "frequency": "once", "date": "2030-01-02", "time": "08:00", "in_this_chat": true, "reply_in": "conv-1"})).await;
    run(dir.path(), SCHEDULE_TASK, &json!({"title": "Other", "prompt": "New chat", "frequency": "daily", "time": "08:00", "reply_in": "conv-1"})).await;
    let tasks = crate::chat::tasks::load(dir.path());
    assert_eq!(
        tasks
            .iter()
            .find(|t| t.title == "Check prices")
            .unwrap()
            .reply_in
            .as_deref(),
        Some("conv-1")
    );
    assert_eq!(
        tasks.iter().find(|t| t.title == "Other").unwrap().reply_in,
        None
    );
}

/// `query_traces` and `get_run`, as the model calls them: list runs, then open one.
#[tokio::test]
async fn the_model_lists_runs_and_opens_one() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    crate::chat::traces::start_run(d, "run-ok", "c1", None, "m/a");
    crate::chat::traces::tool_call(d, "run-ok", "web_search", 12, true);
    crate::chat::traces::end_run(d, "run-ok", "done");
    crate::chat::traces::start_run(d, "run-bad", "c2", None, "m/a");
    crate::chat::traces::tool_call(d, "run-bad", "read_chat", 7, false);
    crate::chat::traces::end_run(d, "run-bad", "error");
    let listed = run(d, QUERY_TRACES, &json!({ "failed_tools": true })).await;
    assert!(!listed.is_error, "{}", listed.text);
    let runs: Value = serde_json::from_str(&listed.text).unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 1);
    let id = runs[0]["id"].as_str().unwrap();
    assert_eq!(id, "run-bad");
    let opened = run(d, GET_RUN, &json!({ "id": id })).await;
    assert!(opened.text.contains("read_chat"), "{}", opened.text);
    assert!(run(d, GET_RUN, &json!({ "id": "nope" })).await.is_error);
}

/// `usage_stats(artifact)`: one thing's use and score.
#[tokio::test]
async fn usage_stats_can_be_read_for_one_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    crate::chat::traces::loaded(d, "skill", &["summarize".into(), "summarize".into()]);
    crate::chat::traces::used(d, "skill", "summarize");
    let out = run(d, USAGE_STATS, &json!({ "artifact": "skill:summarize" })).await;
    assert!(!out.is_error, "{}", out.text);
    let v: Value = serde_json::from_str(&out.text).unwrap();
    assert_eq!(
        (v["loaded"].as_i64(), v["used"].as_i64()),
        (Some(2), Some(1))
    );
    assert!(v["score"].as_f64().unwrap() > 0.0);
    assert!(
        run(d, USAGE_STATS, &json!({ "artifact": "skill:nothing" }))
            .await
            .is_error
    );
    assert!(
        run(d, USAGE_STATS, &json!({ "artifact": "nonsense" }))
            .await
            .is_error
    );
}

#[tokio::test]
async fn memory_tools_save_and_forget() {
    let dir = tempfile::tempdir().unwrap();
    let saved = run(dir.path(), REMEMBER, &json!({"fact": "Lives in Porto"})).await;
    assert!(!saved.is_error, "{}", saved.text);
    // Sensitive data stays out while the switch is on (the default), in facts and pages.
    let refused = run(
        dir.path(),
        REMEMBER,
        &json!({"fact": "The office wifi password is hunter22-blue"}),
    )
    .await;
    assert!(
        refused.is_error && refused.text.contains("a password, key or token"),
        "{}",
        refused.text
    );
    let page = run(
        dir.path(),
        KNOWLEDGE_WRITE,
        &json!({"title": "Payments", "content": "Card 4111 1111 1111 1111"}),
    )
    .await;
    assert!(
        page.is_error && page.text.contains("a card number"),
        "{}",
        page.text
    );
    let memories = memory::load(dir.path());
    assert_eq!(memories.items.len(), 1);
    let short = &memories.items[0].id[..8];
    let forgotten = run(dir.path(), FORGET, &json!({"id": short})).await;
    assert!(!forgotten.is_error, "{}", forgotten.text);
    assert!(memory::load(dir.path()).items.is_empty());
    assert!(
        run(dir.path(), FORGET, &json!({"id": "nope"}))
            .await
            .is_error
    );
}

#[tokio::test]
async fn skills_load_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = run(dir.path(), USE_SKILL, &json!({"name": "write-email"})).await;
    assert!(!loaded.is_error);
    assert!(loaded.text.starts_with("# Skill: write-email"));
    // Not a name: the words search the skills, and nothing matching is an error.
    let found = run(
        dir.path(),
        USE_SKILL,
        &json!({"name": "chart a spreadsheet"}),
    )
    .await;
    assert!(
        !found.is_error && found.text.contains("data-analysis"),
        "{}",
        found.text
    );
    assert!(
        run(dir.path(), USE_SKILL, &json!({"name": "zzqxv"}))
            .await
            .is_error
    );
}

#[test]
fn definitions_follow_settings() {
    let dir = tempfile::tempdir().unwrap();
    let names: Vec<String> = definitions(dir.path())
        .into_iter()
        .map(|t| t.name)
        .filter(|n| {
            n != crate::chat::sandbox::RUN_CODE
                && n != crate::chat::media::GENERATE_IMAGE
                && n != crate::chat::videogen::GENERATE_VIDEO
                && n != crate::chat::animation::MAKE_ANIMATION
                && n != SAVE_SKILL
                && n != NOTIFY
                && n != crate::chat::webpage::BROWSER_VIEW
                && n != crate::chat::webpage::BROWSER_ACT
                && !n.starts_with("knowledge_")
        })
        .collect();
    assert_eq!(
        names,
        [
            WEB_SEARCH,
            UPDATE_PLAN,
            DELEGATE,
            CANVAS,
            SCHEDULE_TASK,
            SEARCH_CHATS,
            READ_CHAT,
            MEMORY_SEARCH,
            REMEMBER,
            FORGET,
            USE_SKILL,
            QUERY_TRACES,
            GET_RUN,
            USAGE_STATS,
            FAILURE_SUMMARY
        ]
    );
    memory::save(
        dir.path(),
        &memory::Memories {
            enabled: false,
            ..Default::default()
        },
    )
    .unwrap();
    let names: Vec<String> = definitions(dir.path())
        .into_iter()
        .map(|t| t.name)
        .filter(|n| {
            n != crate::chat::sandbox::RUN_CODE
                && n != crate::chat::media::GENERATE_IMAGE
                && n != crate::chat::videogen::GENERATE_VIDEO
                && n != crate::chat::animation::MAKE_ANIMATION
                && n != SAVE_SKILL
                && n != NOTIFY
                && n != crate::chat::webpage::BROWSER_VIEW
                && n != crate::chat::webpage::BROWSER_ACT
                && !n.starts_with("knowledge_")
        })
        .collect();
    assert_eq!(
        names,
        [
            WEB_SEARCH,
            UPDATE_PLAN,
            DELEGATE,
            CANVAS,
            SCHEDULE_TASK,
            USE_SKILL,
            QUERY_TRACES,
            GET_RUN,
            USAGE_STATS,
            FAILURE_SUMMARY
        ]
    );
}

#[tokio::test]
async fn knowledge_can_be_used_without_being_changed() {
    let dir = tempfile::tempdir().unwrap();
    memory::save(
        dir.path(),
        &memory::Memories {
            update: false,
            ..Default::default()
        },
    )
    .unwrap();
    let names: Vec<String> = definitions(dir.path())
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(
        names.iter().any(|n| n == KNOWLEDGE_READ) && names.iter().any(|n| n == KNOWLEDGE_SEARCH)
    );
    assert!(!names
        .iter()
        .any(|n| n == REMEMBER || n == FORGET || n == KNOWLEDGE_WRITE));
    let refused = run(dir.path(), REMEMBER, &json!({"fact": "Lives in Porto"})).await;
    assert!(refused.is_error && memory::load(dir.path()).items.is_empty());
    let prompt = memory::prompt_for(&memory::load(dir.path()), "", None, None).unwrap();
    assert!(prompt.contains("turned off saving") && !prompt.contains("`remember`"));
}

#[tokio::test]
async fn earlier_chats_are_found_and_read() {
    use rig_core::completion::Message;
    let dir = tempfile::tempdir().unwrap();
    let mut c = crate::chat::store::Conversation::new("c1".into(), None, None);
    c.title = "Kitchen renovation".into();
    c.messages = vec![
        Message::user("Which oak worktop should we pick?"),
        Message::assistant("The 40 mm solid oak one, oiled."),
    ];
    crate::chat::store::save_conversation(dir.path(), &c).unwrap();
    let found = run(dir.path(), SEARCH_CHATS, &json!({"query": "oak worktop"})).await;
    assert!(
        found.text.contains("\"c1\"") && found.text.contains("Kitchen renovation"),
        "{}",
        found.text
    );
    // Not every word needs to match.
    assert!(run(
        dir.path(),
        SEARCH_CHATS,
        &json!({"query": "walnut worktop"})
    )
    .await
    .text
    .contains("c1"));
    let read = run(dir.path(), READ_CHAT, &json!({"id": "c1"})).await;
    assert!(read.text.contains("User: Which oak") && read.text.contains("Assistant: The 40 mm"));
    assert!(
        run(dir.path(), READ_CHAT, &json!({"id": "nope"}))
            .await
            .is_error
    );
}
