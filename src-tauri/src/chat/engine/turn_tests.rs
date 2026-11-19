//! Whole turns against scripted models, through a test [`Host`]: what the engine streams, which
//! tools it runs, what it asks the user, and how it recovers. They pin today's behaviour so the
//! loop can be taken apart safely.

use std::sync::Arc;

use rig_core::completion::Usage;
use rig_core::test_utils::{mock_final, MockCompletionModel, MockError, MockStreamEvent};
use serde_json::json;

use super::*;
use crate::chat::host::testing::TestHost;

const MODEL: &str = "mock/main";
const BACKUP: &str = "mock/backup";

fn request(text: &str, approval: ApprovalMode) -> TurnRequest {
    TurnRequest {
        conversation_id: "chat-1".into(),
        text: text.into(),
        model: MODEL.into(),
        speed: Speed::Fast,
        approval,
        project_id: None,
        images: Vec::new(),
        edit_index: None,
        temporary: false,
        agent_id: None,
        resume: false,
        workdir: None,
        connectors: None,
        route_note: None,
        tools_off: Vec::new(),
        max_steps: None,
        only_tools: None,
        system: None,
        skills_off: Vec::new(),
    }
}

fn end() -> MockStreamEvent {
    MockStreamEvent::FinalResponse(mock_final(Usage::default()))
}

fn says(text: &str) -> Vec<MockStreamEvent> {
    vec![MockStreamEvent::text(text), end()]
}

fn calls(name: &str, arguments: serde_json::Value) -> Vec<MockStreamEvent> {
    vec![MockStreamEvent::tool_call("call-1", name, arguments), end()]
}

/// A host serving `turns` from the main model; the mock is returned to read what it was sent.
fn host_with(turns: Vec<Vec<MockStreamEvent>>) -> (Arc<TestHost>, MockCompletionModel) {
    let host = Arc::new(TestHost::new());
    let model = MockCompletionModel::from_stream_turns(turns);
    host.model(MODEL, model.clone().erase());
    (host, model)
}

async fn run(host: &Arc<TestHost>, request: &TurnRequest) -> Result<bool, String> {
    let cancel = host
        .state
        .begin(&request.conversation_id)
        .expect("no turn running");
    let dyn_host: Arc<dyn Host> = host.clone();
    let outcome = turn(&dyn_host, request, cancel).await;
    host.state.finish(&request.conversation_id);
    outcome
}

fn saved(host: &TestHost) -> Conversation {
    store::load_conversation(host.dir.path(), "chat-1").expect("conversation saved")
}

fn tool_ends(host: &TestHost) -> Vec<(String, bool)> {
    host.events()
        .into_iter()
        .filter_map(|e| match e {
            ChatEvent::ToolEnd {
                output, is_error, ..
            } => Some((output, is_error)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_plain_answer_streams_and_is_saved() {
    let (host, model) = host_with(vec![says("Hello there.")]);
    assert_eq!(
        run(&host, &request("Hi", ApprovalMode::Ask)).await,
        Ok(false)
    );
    assert_eq!(host.streamed_text(), "Hello there.");
    let c = saved(&host);
    assert_eq!(c.messages.len(), 2);
    assert_eq!(context::message_text(&c.messages[1]), "Hello there.");
    // The model was offered the built-in tools and told the date.
    let sent = &model.requests()[0];
    assert!(sent.tools.iter().any(|t| t.name == builtins::WEB_SEARCH));
    assert!(serde_json::to_string(sent).unwrap().contains("Today is"));
}

#[tokio::test]
async fn a_tool_call_runs_and_its_result_goes_back_to_the_model() {
    let (host, model) = host_with(vec![
        calls(builtins::UPDATE_PLAN, json!({ "steps": ["Look", "Fix"] })),
        says("Planned."),
    ]);
    assert_eq!(
        run(&host, &request("Plan it", ApprovalMode::Ask)).await,
        Ok(false)
    );
    assert_eq!(
        tool_ends(&host),
        vec![("Plan shown to the user (2 steps).".to_string(), false)]
    );
    assert_eq!(host.streamed_text(), "Planned.");
    // The second request carried the tool's result.
    let second = serde_json::to_string(&model.requests()[1].chat_history).unwrap();
    assert!(second.contains("Plan shown to the user"), "{second}");
    // prompt, call, result, answer
    assert_eq!(saved(&host).messages.len(), 4);
}

#[tokio::test]
async fn a_change_asks_in_ask_mode_and_a_declined_one_does_not_run() {
    let schedule =
        json!({ "title": "News", "prompt": "Brief me", "frequency": "daily", "time": "09:00" });
    let (host, _) = host_with(vec![
        calls(builtins::SCHEDULE_TASK, schedule),
        says("Understood."),
    ]);
    assert_eq!(
        run(&host, &request("Every day", ApprovalMode::Ask)).await,
        Ok(false)
    );
    assert_eq!(
        host.approvals.lock().unwrap().clone(),
        vec![builtins::SCHEDULE_TASK.to_string()]
    );
    let ends = tool_ends(&host);
    assert!(ends[0].1 && ends[0].0.contains("declined"), "{ends:?}");
    assert!(tasks::load(host.dir.path()).is_empty());
}

#[tokio::test]
async fn plan_mode_refuses_a_change_without_asking() {
    let schedule =
        json!({ "title": "News", "prompt": "Brief me", "frequency": "daily", "time": "09:00" });
    let (host, _) = host_with(vec![
        calls(builtins::SCHEDULE_TASK, schedule),
        says("Here is the plan."),
    ]);
    assert_eq!(
        run(&host, &request("Every day", ApprovalMode::Plan)).await,
        Ok(false)
    );
    assert!(host.approvals.lock().unwrap().is_empty());
    let ends = tool_ends(&host);
    assert!(ends[0].1 && ends[0].0.starts_with("Plan mode"), "{ends:?}");
    assert!(tasks::load(host.dir.path()).is_empty());
}

#[tokio::test]
async fn guidance_sent_during_a_turn_joins_the_conversation() {
    let (host, model) = host_with(vec![says("Noted both.")]);
    let request = request("Write a haiku", ApprovalMode::Ask);
    let cancel = host.state.begin("chat-1").unwrap();
    assert!(host.state.steer("chat-1", "about autumn".into()));
    let dyn_host: Arc<dyn Host> = host.clone();
    assert_eq!(turn(&dyn_host, &request, cancel).await, Ok(false));
    host.state.finish("chat-1");
    assert!(host
        .events()
        .iter()
        .any(|e| matches!(e, ChatEvent::Steered { text, .. } if text == "about autumn")));
    let sent = serde_json::to_string(&model.requests()[0]).unwrap();
    assert!(
        sent.contains("(Sent while you were working) about autumn"),
        "{sent}"
    );
}

#[tokio::test]
async fn a_failing_model_hands_over_to_the_backup_once() {
    let (host, _) = host_with(vec![vec![MockStreamEvent::Error(MockError::provider(
        "the provider is down",
    ))]]);
    host.model(
        BACKUP,
        MockCompletionModel::from_stream_turns(vec![says("From the backup.")]).erase(),
    );
    providers::save(
        host.dir.path(),
        &providers::ProvidersFile {
            backup_model: Some(BACKUP.into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        run(&host, &request("Hi", ApprovalMode::Ask)).await,
        Ok(false)
    );
    assert_eq!(host.streamed_text(), "From the backup.");
    assert!(host.events().iter().any(
        |e| matches!(e, ChatEvent::Notice { text, .. } if text.contains("continuing with backup"))
    ));
}

#[tokio::test]
async fn a_failing_model_without_a_backup_ends_the_turn_with_its_error() {
    let (host, _) = host_with(vec![vec![MockStreamEvent::Error(MockError::provider(
        "the provider is down",
    ))]]);
    let outcome = run(&host, &request("Hi", ApprovalMode::Ask)).await;
    assert!(
        outcome
            .as_ref()
            .is_err_and(|e| e.contains("the provider is down")),
        "{outcome:?}"
    );
    // The user's message is kept, so the turn can be resumed.
    assert_eq!(saved(&host).messages.len(), 1);
}

/// A conversation past the compaction threshold whose summary fails is not summarized again on
/// every step of the same turn.
#[tokio::test]
async fn a_failed_summary_is_tried_once_per_turn() {
    let (host, _) = host_with(vec![
        calls(builtins::UPDATE_PLAN, json!({ "steps": ["One"] })),
        says("Done."),
    ]);
    let mut c = Conversation::new("chat-1".into(), None, Some(MODEL.into()));
    for i in 0..3 {
        c.messages.push(Message::user(format!("question {i}")));
        c.messages.push(Message::assistant(format!("answer {i}")));
    }
    c.usage.context_tokens = 10_000_000;
    store::save_conversation(host.dir.path(), &c).unwrap();
    assert_eq!(
        run(&host, &request("And now?", ApprovalMode::Ask)).await,
        Ok(false)
    );
    let compactions = host
        .events()
        .iter()
        .filter(|e| matches!(e, ChatEvent::Compacting { .. }))
        .count();
    assert_eq!(compactions, 1);
    assert_eq!(host.streamed_text(), "Done.");
}

/// A `delegate` worker runs through the same loop as the chat: its tool calls pass the same
/// hooks and land in the activity log, and its report comes back to the main model.
#[tokio::test]
async fn a_delegate_worker_runs_through_the_same_loop() {
    let delegate =
        json!({ "tasks": [{ "title": "Find the release date", "instructions": "Look it up." }] });
    let (host, model) = host_with(vec![
        // The chat delegates...
        calls(builtins::DELEGATE, delegate),
        // ...the worker calls a tool, then reports...
        calls(builtins::USE_SKILL, json!({ "name": "nope" })),
        says("The release is planned for March, according to the project's own roadmap page."),
        // ...and the chat answers from the report.
        says("It ships in March."),
    ]);
    assert_eq!(
        run(&host, &request("When is it out?", ApprovalMode::Ask)).await,
        Ok(false)
    );
    assert_eq!(host.streamed_text(), "It ships in March.");
    let report = serde_json::to_string(&model.requests()[3].chat_history).unwrap();
    assert!(report.contains("planned for March"), "{report}");
    let logged: Vec<String> = super::super::audit::entries(host.dir.path(), None)
        .into_iter()
        .map(|e| e.tool)
        .collect();
    assert!(
        logged.contains(&builtins::USE_SKILL.to_string()),
        "{logged:?}"
    );
    assert!(host
        .events()
        .iter()
        .any(|e| matches!(e, ChatEvent::Subagent { status, .. } if status == "done")));
    // The worker's conversation is not kept.
    let kept = std::fs::read_dir(store::chat_dir(host.dir.path()).join("conversations"))
        .unwrap()
        .count();
    assert_eq!(kept, 1);
}

/// Each turn leaves a trace: the run, its model calls and its tool calls, which the model can
/// read back with `usage_stats`.
#[tokio::test]
async fn a_turn_is_traced_and_the_model_can_read_the_trace() {
    let (host, _) = host_with(vec![
        calls(builtins::UPDATE_PLAN, json!({ "steps": ["One"] })),
        calls(builtins::USAGE_STATS, json!({ "days": 1 })),
        says("Done."),
    ]);
    assert_eq!(
        run(&host, &request("Plan, then check", ApprovalMode::Ask)).await,
        Ok(false)
    );
    let usage =
        super::super::traces::usage(host.dir.path(), Utc::now() - chrono::Duration::days(1));
    assert_eq!(usage.runs, 1);
    assert_eq!(usage.models[0].model, MODEL);
    assert_eq!(usage.models[0].calls, 3);
    assert!(usage
        .tools
        .iter()
        .any(|t| t.tool == builtins::UPDATE_PLAN && t.calls == 1));
    // The usage_stats call itself read the trace, which by then held the plan's call.
    let ends = tool_ends(&host);
    assert!(ends[1].0.contains("update_plan"), "{ends:?}");
}

/// The kernel's tool-creation loop: the model writes a tool in one chat, the user approves it,
/// and another chat is offered it and runs it in the sandbox.
#[tokio::test]
async fn a_tool_written_in_one_chat_is_approved_and_used_in_another() {
    use crate::chat::script_tools::{CREATE_TOOL, PROMOTE_TOOL};
    let script =
        "function main(a) { return { words: a.text.split(/\\s+/).filter(Boolean).length }; }";
    let (host, model) = host_with(vec![
        // Chat 1: write it, then ask for it to be promoted.
        calls(
            CREATE_TOOL,
            json!({
                "name": "count_words",
                "description": "Count the words in a text.",
                "parameters": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] },
                "script": script,
                "test_args": { "text": "a b c" }
            }),
        ),
        calls(PROMOTE_TOOL, json!({ "name": "count_words" })),
        says("The tool is ready."),
        // Chat 2: it is on offer, and runs.
        calls(
            "script__count_words",
            json!({ "text": "one two three four" }),
        ),
        says("Four words."),
    ]);
    *host.answer.lock().unwrap() = ApprovalDecision::Once;
    assert_eq!(
        run(&host, &request("Make a word counter", ApprovalMode::Ask)).await,
        Ok(false)
    );
    assert_eq!(
        host.approvals.lock().unwrap().clone(),
        vec![PROMOTE_TOOL.to_string()],
        "promotion asks, even though creating did not"
    );
    let mut second = request("How many words in: one two three four", ApprovalMode::Ask);
    second.conversation_id = "chat-2".into();
    assert_eq!(run(&host, &second).await, Ok(false));
    // The second chat's requests offered it (the review before promotion is a request too).
    assert!(model.requests().iter().any(|r| r
        .tools
        .iter()
        .any(|t| t.name.as_str() == "script__count_words")));
    let ends = tool_ends(&host);
    assert_eq!(ends.last().unwrap(), &(r#"{"words":4}"#.to_string(), false));
}

/// Declined, the tool stays a candidate and is never offered.
#[tokio::test]
async fn a_declined_tool_is_never_offered() {
    use crate::chat::script_tools::{CREATE_TOOL, PROMOTE_TOOL};
    let (host, model) = host_with(vec![
        calls(
            CREATE_TOOL,
            json!({ "name": "noop_tool", "description": "Does nothing.", "script": "function main() { return 1; }" }),
        ),
        calls(PROMOTE_TOOL, json!({ "name": "noop_tool" })),
        says("Understood."),
        says("Hello."),
    ]);
    assert_eq!(
        run(&host, &request("Make a tool", ApprovalMode::Auto)).await,
        Ok(false)
    );
    let ends = tool_ends(&host);
    assert!(ends[1].1 && ends[1].0.contains("declined"), "{ends:?}");
    let mut second = request("Hi", ApprovalMode::Auto);
    second.conversation_id = "chat-2".into();
    assert_eq!(run(&host, &second).await, Ok(false));
    assert!(!model.requests().iter().any(|r| r
        .tools
        .iter()
        .any(|t| t.name.as_str().starts_with("script__"))));
}

/// An eval suite runs its cases through the same loop, grades them, and saves the run; nothing a
/// case does is kept.
#[tokio::test]
async fn an_eval_suite_runs_through_the_loop_and_is_graded() {
    use crate::chat::evals;
    let (host, _) = host_with(vec![
        calls(
            builtins::UPDATE_PLAN,
            json!({ "steps": ["Back up", "Move", "Check"] }),
        ),
        says("Back up first, then move the files, then check the DNS."),
        says("Just move it."),
    ]);
    let case = |id: &str| evals::Case {
        id: id.into(),
        prompt: "Plan the website move.".into(),
        tools: vec![builtins::UPDATE_PLAN.into()],
        facts: vec!["back up".into()],
        ..evals::Case::default()
    };
    evals::add_case(host.dir.path(), "plans", &case("a-plan-first")).unwrap();
    evals::add_case(host.dir.path(), "plans", &case("b-plan-again")).unwrap();
    let dyn_host: Arc<dyn Host> = host.clone();
    let run = evals::run_suite(&dyn_host, "plans", MODEL, &evals::Without::default())
        .await
        .unwrap();
    assert!(run.cases[0].passed, "{:?}", run.cases[0]);
    assert!(!run.cases[1].passed);
    assert_eq!(run.cases[1].score, 0.0);
    assert_eq!(run.score, 0.5);
    // The run is kept beside the suite; the cases' conversations are not.
    let runs = std::fs::read_dir(evals::evals_dir(host.dir.path()).join("plans").join("runs"))
        .unwrap()
        .count();
    assert_eq!(runs, 1);
    let kept = std::fs::read_dir(store::chat_dir(host.dir.path()).join("conversations"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(kept, 0);
}

/// With the user's eval suites, a promotion measures the suite without and with the tool, and a
/// tool that makes it worse goes back to being a candidate.
#[tokio::test]
async fn a_promotion_is_gated_on_the_user_s_evals() {
    use crate::chat::{evals, script_tools};
    let gated = |base_answer: &'static str, with_answer: &'static str| async move {
        let (host, _) = host_with(vec![
            calls(
                script_tools::CREATE_TOOL,
                json!({ "name": "counter", "description": "Counts.", "script": "function main() { return 4; }" }),
            ),
            calls(script_tools::PROMOTE_TOOL, json!({ "name": "counter" })),
            // The suite without the tool, then with it.
            says(base_answer),
            says(with_answer),
            says("Done."),
        ]);
        *host.answer.lock().unwrap() = ApprovalDecision::Once;
        evals::add_case(
            host.dir.path(),
            "counting",
            &evals::Case {
                id: "count".into(),
                prompt: "How many?".into(),
                facts: vec!["4 words".into()],
                ..evals::Case::default()
            },
        )
        .unwrap();
        assert_eq!(
            run(&host, &request("Make a counter", ApprovalMode::Ask)).await,
            Ok(false)
        );
        let ends = tool_ends(&host);
        let active = script_tools::load(host.dir.path(), "counter").map(|t| t.status);
        (ends[1].clone(), active)
    };
    let ((text, failed), status) = gated("I don't know.", "It has 4 words.").await;
    assert!(!failed && text.contains("did not get worse"), "{text}");
    assert_eq!(status, Some(script_tools::Status::Active));

    let ((text, failed), status) = gated("It has 4 words.", "I don't know.").await;
    assert!(
        failed && text.contains("made the `counting` eval suite worse"),
        "{text}"
    );
    assert_eq!(status, Some(script_tools::Status::Candidate));
}

/// A proposed change is reviewed, shown to the user with the review, and applied only on yes.
#[tokio::test]
async fn a_proposed_change_is_reviewed_and_applied_only_when_approved() {
    use crate::chat::proposals::PROPOSE_CHANGE;
    let proposal = json!({
        "summary": "Remember the user's time zone",
        "evidence": "They gave it twice this week",
        "change": { "layer": "memory", "fact": "Works on Lisbon time" }
    });
    let (host, _) = host_with(vec![
        calls(PROPOSE_CHANGE, proposal.clone()),
        says("Noted."),
    ]);
    *host.answer.lock().unwrap() = ApprovalDecision::Once;
    assert_eq!(
        run(&host, &request("Remember that", ApprovalMode::Auto)).await,
        Ok(false)
    );
    // It asked even in Full auto, and the scripted model gave no review, which the card says.
    assert_eq!(
        host.approvals.lock().unwrap().clone(),
        vec![PROPOSE_CHANGE.to_string()]
    );
    let ends = tool_ends(&host);
    assert!(!ends[0].1 && ends[0].0.contains("Reviewer"), "{ends:?}");
    assert!(super::super::memory::load(host.dir.path())
        .items
        .iter()
        .any(|m| m.text == "Works on Lisbon time"));

    let (declined, _) = host_with(vec![calls(PROPOSE_CHANGE, proposal), says("Fine.")]);
    assert_eq!(
        run(&declined, &request("Remember that", ApprovalMode::Auto)).await,
        Ok(false)
    );
    assert!(super::super::memory::load(declined.dir.path())
        .items
        .is_empty());
}

/// The turn records the prompt's parts and their sizes, which the context view shows.
#[tokio::test]
async fn the_prompt_s_parts_are_recorded_for_the_context_view() {
    let (host, _) = host_with(vec![says("Hello.")]);
    let mut c = Conversation::new("chat-1".into(), None, Some(MODEL.into()));
    c.params.instructions = "Answer in haiku.".into();
    store::save_conversation(host.dir.path(), &c).unwrap();
    assert_eq!(
        run(&host, &request("Hi", ApprovalMode::Plan)).await,
        Ok(false)
    );
    let names: Vec<String> = saved(&host)
        .prompt_parts
        .into_iter()
        .map(|p| p.name)
        .collect();
    for part in [
        "identity",
        "chat instructions",
        "plan mode",
        "how to use tools",
    ] {
        assert!(names.contains(&part.to_string()), "{names:?}");
    }
}

/// The nightly pass runs the suites that have expectations and compares each with its last run.
#[tokio::test]
async fn nightly_evals_compare_each_suite_with_its_last_run() {
    use crate::chat::evals;
    let (host, _) = host_with(vec![says("It is 4 words."), says("No idea.")]);
    evals::add_case(
        host.dir.path(),
        "words",
        &evals::Case {
            id: "four".into(),
            prompt: "How many?".into(),
            facts: vec!["4 words".into()],
            ..evals::Case::default()
        },
    )
    .unwrap();
    // A suite still waiting for expectations is not run.
    evals::add_case(
        host.dir.path(),
        evals::REGRESSIONS,
        &evals::Case {
            id: "waiting".into(),
            prompt: "x".into(),
            needs_expectation: true,
            ..evals::Case::default()
        },
    )
    .unwrap();
    let dyn_host: Arc<dyn Host> = host.clone();
    let first = evals::nightly(&dyn_host, MODEL).await;
    assert_eq!(first.len(), 1);
    assert_eq!((first[0].score, first[0].before), (1.0, None));
    let second = evals::nightly(&dyn_host, MODEL).await;
    assert_eq!((second[0].score, second[0].before), (0.0, Some(1.0)));
    assert_eq!(second[0].worse, vec!["four".to_string()]);
}

/// The context manager's operations: past 30 connector tools the rest wait to be loaded;
/// `find_tools` loads, `unload_tools` unloads, `context_status` reports the budget.
#[tokio::test]
async fn tools_load_and_unload_on_demand_and_the_budget_can_be_read() {
    use crate::tools::registry::testing::FakeConnector;
    let names: Vec<String> = (0..31).map(|i| format!("report_{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (host, model) = host_with(vec![
        calls(deferred::FIND_TOOLS, json!({ "query": "report_7" })),
        calls(
            handlers::UNLOAD_TOOLS,
            json!({ "names": ["mcp-many__report_7"] }),
        ),
        calls(handlers::CONTEXT_STATUS, json!({})),
        says("Done."),
    ]);
    host.registry
        .register(FakeConnector::new("mcp-many").listing(&refs));
    assert_eq!(
        run(&host, &request("Use the reports", ApprovalMode::Ask)).await,
        Ok(false)
    );
    let sent = |i: usize| -> Vec<String> {
        model.requests()[i]
            .tools
            .iter()
            .map(|t| t.name.to_string())
            .collect()
    };
    // Waiting tools are not sent until found; once found, they are; once unloaded, not again.
    assert!(!sent(0).contains(&"mcp-many__report_7".to_string()));
    assert!(
        sent(1).contains(&"mcp-many__report_7".to_string()),
        "{:?}",
        sent(1)
    );
    assert!(!sent(2).contains(&"mcp-many__report_7".to_string()));
    let ends = tool_ends(&host);
    assert!(
        ends[2].0.contains("\"window\"") && ends[2].0.contains("\"instructions\""),
        "{ends:?}"
    );
    let loaded = saved(&host).loaded_tools;
    assert!(!loaded.is_empty(), "find_tools loaded its matches");
    assert!(
        !loaded.contains(&"mcp-many__report_7".to_string()),
        "{loaded:?}"
    );
}
