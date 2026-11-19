//! The evaluation harness: the only ground truth for whether a change made the assistant better.
//!
//! A suite is a folder of cases, `<config>/evals/<suite>/cases/<id>.json`, outside `chat/`, where
//! no tool writes: the user owns them. A case is a prompt with what a good answer does: the tools
//! it must call (`tools`), must not call (`not_tools`), and the facts its answer must contain
//! (`facts`, case-insensitive). Each case runs through the app's own agent loop, unattended (so
//! nothing it does can change anything) in a temporary conversation deleted after.
//!
//! A run is saved to `<suite>/runs/<time>.json`. [`compare`] tells which cases got better or
//! worse between two runs, and [`ablate`] reruns a suite with one tool or skill left out, which is
//! how to tell whether that artifact helps. An answer the user rates down becomes a case in the
//! `regressions` suite, waiting for the user to say what the right answer contains.
//!
//! ```text
//! cortex-desktop eval tool-selection [--model vendor/model] [--without tool:web_search]
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

pub use cortex_kernel::evals::*;

use super::engine::{ChatEvent, TurnRequest};
use super::host::Host;
use super::tools::{ApprovalMode, ChatTool};
use crate::tools::registry::ApprovalDecision;

pub const REGRESSIONS: &str = "regressions";
pub const STARTER: &str = "tool-selection";

/// Where suites live: beside the app's data, not inside `chat/`, which tools write.
pub fn evals_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("evals")
}

fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 60
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn suite_dir(config_dir: &Path, suite: &str) -> Result<PathBuf, String> {
    if !valid(suite) {
        return Err(format!("`{suite}` is not a suite name."));
    }
    Ok(evals_dir(config_dir).join(suite))
}

pub fn suites(config_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(evals_dir(config_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("cases").is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    out.sort();
    out
}

pub fn cases(config_dir: &Path, suite: &str) -> Vec<Case> {
    let Ok(dir) = suite_dir(config_dir, suite) else {
        return Vec::new();
    };
    let mut out: Vec<Case> = std::fs::read_dir(dir.join("cases"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let text = crate::chat::at_rest::read_to_string(e.path()).ok()?;
            let mut case: Case = serde_json::from_str(&text).ok()?;
            if case.id.is_empty() {
                case.id = e.path().file_stem()?.to_string_lossy().into_owned();
            }
            Some(case)
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub fn add_case(config_dir: &Path, suite: &str, case: &Case) -> Result<(), String> {
    if !valid(&case.id) {
        return Err(format!("`{}` is not a case id.", case.id));
    }
    let dir = suite_dir(config_dir, suite)?.join("cases");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(
        dir.join(format!("{}.json", case.id)),
        serde_json::to_string_pretty(case).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// A suite as the user edits it: its cases and how its latest full run scored.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteView {
    pub suite: String,
    pub cases: Vec<Case>,
    pub last_score: Option<f64>,
}

pub fn suite_views(config_dir: &Path) -> Vec<SuiteView> {
    suites(config_dir)
        .into_iter()
        .map(|suite| SuiteView {
            cases: cases(config_dir, &suite),
            last_score: last_run(config_dir, &suite)
                .filter(|r| r.without.is_empty())
                .map(|r| r.score),
            suite,
        })
        .collect()
}

/// The user writes or edits a case. Once it says what a good answer does, a case saved from a
/// rating stops waiting and runs.
pub fn save_case(config_dir: &Path, suite: &str, mut case: Case) -> Result<(), String> {
    let clean = |items: &mut Vec<String>| {
        for item in items.iter_mut() {
            *item = item.trim().to_string();
        }
        items.retain(|i| !i.is_empty());
    };
    clean(&mut case.tools);
    clean(&mut case.not_tools);
    clean(&mut case.facts);
    case.prompt = case.prompt.trim().to_string();
    if case.prompt.is_empty() {
        return Err("A case needs the prompt to send.".into());
    }
    if case.checks() > 0 {
        case.needs_expectation = false;
    }
    add_case(config_dir, suite, &case)
}

pub fn delete_case(config_dir: &Path, suite: &str, id: &str) -> Result<(), String> {
    if !valid(id) {
        return Err(format!("`{id}` is not a case id."));
    }
    let path = suite_dir(config_dir, suite)?
        .join("cases")
        .join(format!("{id}.json"));
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Writes the starter suite, three tool-selection cases, unless it exists.
pub fn ensure_starter(config_dir: &Path) -> Result<(), String> {
    if !cases(config_dir, STARTER).is_empty() {
        return Ok(());
    }
    let case = |id: &str, prompt: &str, tools: &[&str]| Case {
        id: id.into(),
        prompt: prompt.into(),
        tools: tools.iter().map(|t| t.to_string()).collect(),
        ..Case::default()
    };
    for c in [
        case(
            "current-facts-search-the-web",
            "What is the latest stable version of the Rust compiler?",
            &[super::builtins::WEB_SEARCH],
        ),
        case(
            "earlier-chats-are-searched",
            "What did we decide earlier about the trip to Porto?",
            &[super::builtins::SEARCH_CHATS],
        ),
        case(
            "several-steps-get-a-plan",
            "Plan, step by step, how I should migrate a small website to a new host this weekend.",
            &[super::builtins::UPDATE_PLAN],
        ),
    ] {
        add_case(config_dir, STARTER, &c)?;
    }
    Ok(())
}

/// Runs one case's turn through `parent`'s models and data, recording the tools it calls.
struct RecordingHost {
    parent: Arc<dyn Host>,
    state: super::ChatState,
    called: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Host for RecordingHost {
    fn config_dir(&self) -> Result<PathBuf, String> {
        self.parent.config_dir()
    }

    fn emit(&self, event: ChatEvent) {
        if let ChatEvent::ToolStart { name, .. } = event {
            self.called
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(name);
        }
    }

    fn chat_state(&self) -> &super::ChatState {
        &self.state
    }

    fn registry(&self) -> &crate::tools::registry::ToolRegistry {
        self.parent.registry()
    }

    async fn connect_model(
        &self,
        reference: &str,
    ) -> Result<rig_core::DynModel<rig_core::operation::Completion>, String> {
        self.parent.connect_model(reference).await
    }

    async fn approve(&self, _: &ChatTool, _: &Value, _: Option<&str>) -> ApprovalDecision {
        // An eval never changes anything; the run is unattended, so nothing gets this far.
        ApprovalDecision::Deny
    }

    fn notify(&self, _title: &str, _body: &str) {}
}

pub async fn run_case(
    host: &Arc<dyn Host>,
    model: &str,
    case: &Case,
    without: &Without,
) -> CaseResult {
    if case.needs_expectation || case.checks() == 0 {
        return grade(case, &[], "");
    }
    let Ok(config_dir) = host.config_dir() else {
        return grade(case, &[], "");
    };
    let id = format!("eval-{}", uuid::Uuid::new_v4().simple());
    let mut conversation = super::store::Conversation::new(id.clone(), None, Some(model.into()));
    conversation.temporary = true;
    conversation.title = format!("Eval: {}", case.id);
    if super::store::save_conversation(&config_dir, &conversation).is_err() {
        return grade(case, &[], "");
    }
    let recorder = Arc::new(RecordingHost {
        parent: host.clone(),
        state: super::ChatState::default(),
        called: Mutex::new(Vec::new()),
    });
    let request = TurnRequest {
        conversation_id: id.clone(),
        text: case.prompt.clone(),
        model: model.into(),
        approval: ApprovalMode::Unattended,
        temporary: true,
        tools_off: without.tools.clone(),
        skills_off: without.skills.clone(),
        ..TurnRequest::default()
    };
    let answer = match recorder.state.begin(&id) {
        Some(cancel) => {
            let dyn_host: Arc<dyn Host> = recorder.clone();
            let outcome = super::engine::turn(&dyn_host, &request, cancel).await;
            recorder.state.finish(&id);
            match outcome {
                Ok(_) => super::store::load_conversation(&config_dir, &id)
                    .ok()
                    .and_then(|c| {
                        c.messages
                            .last()
                            .filter(|m| matches!(m, rig_core::completion::Message::Assistant(_)))
                            .map(super::context::message_text)
                    })
                    .unwrap_or_default(),
                Err(e) => format!("(the turn failed: {e})"),
            }
        }
        None => String::new(),
    };
    let _ = super::store::delete_conversation(&config_dir, &id);
    let called = recorder
        .called
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    grade(case, &called, &answer)
}

/// Runs every case of `suite` on `model`, saves the run, and returns it.
pub async fn run_suite(
    host: &Arc<dyn Host>,
    suite: &str,
    model: &str,
    without: &Without,
) -> Result<SuiteRun, String> {
    let config_dir = host.config_dir()?;
    let all = cases(&config_dir, suite);
    if all.is_empty() {
        return Err(format!("The suite `{suite}` has no cases."));
    }
    let mut results = Vec::new();
    for case in &all {
        results.push(run_case(host, model, case, without).await);
    }
    let ran: Vec<&CaseResult> = results.iter().filter(|r| !r.skipped).collect();
    let score = if ran.is_empty() {
        0.0
    } else {
        ran.iter().map(|r| r.score).sum::<f64>() / ran.len() as f64
    };
    let run = SuiteRun {
        suite: suite.into(),
        at: Utc::now(),
        model: model.into(),
        without: without.labels(),
        score,
        cases: results,
    };
    let dir = suite_dir(&config_dir, suite)?.join("runs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = run.at.format("%Y%m%dT%H%M%S%3fZ");
    crate::chat::at_rest::write(
        dir.join(format!("{stamp}.json")),
        serde_json::to_string_pretty(&run).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(run)
}

/// The latest saved run of `suite`, if any.
pub fn last_run(config_dir: &Path, suite: &str) -> Option<SuiteRun> {
    let dir = suite_dir(config_dir, suite).ok()?.join("runs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let text = crate::chat::at_rest::read_to_string(files.last()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// One suite's nightly result: its score, and the cases that got worse since the last full run.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Nightly {
    pub suite: String,
    pub score: f64,
    pub before: Option<f64>,
    pub worse: Vec<String>,
}

/// Runs every suite that has cases to check, on `model`, and compares each with its last run.
pub async fn nightly(host: &Arc<dyn Host>, model: &str) -> Vec<Nightly> {
    let Ok(config_dir) = host.config_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for suite in suites(&config_dir) {
        if !cases(&config_dir, &suite)
            .iter()
            .any(|c| !c.needs_expectation)
        {
            continue;
        }
        let previous = last_run(&config_dir, &suite).filter(|r| r.without.is_empty());
        let Ok(run) = run_suite(host, &suite, model, &Without::default()).await else {
            continue;
        };
        let comparison = previous.as_ref().map(|p| compare(p, &run));
        out.push(Nightly {
            suite,
            score: run.score,
            before: previous.map(|p| p.score),
            worse: comparison.map(|c| c.worse).unwrap_or_default(),
        });
    }
    out
}

/// The notice for a nightly result whose score dropped since the last full run, if it did.
pub fn drop_notice(result: &Nightly) -> Option<(&'static str, String)> {
    let before = result.before?;
    (result.score + 1e-9 < before).then(|| {
        (
            "An eval score dropped",
            format!(
                "{}: {:.2} (was {:.2}). Worse: {}",
                result.suite,
                result.score,
                before,
                result.worse.join(", ")
            ),
        )
    })
}

/// Runs a suite with everything, then without `left_out`. If the score holds without it, the
/// artifact does not help on this suite.
pub async fn ablate(
    host: &Arc<dyn Host>,
    suite: &str,
    model: &str,
    left_out: &Without,
) -> Result<(SuiteRun, SuiteRun, Comparison), String> {
    let with = run_suite(host, suite, model, &Without::default()).await?;
    let without = run_suite(host, suite, model, left_out).await?;
    let comparison = compare(&with, &without);
    Ok((with, without, comparison))
}

/// A rated-down answer becomes a case in the regressions suite: the prompt that led to it, and
/// the answer that was wrong in its note. It waits for the user to add what a right answer does.
pub fn regression_from_rating(
    config_dir: &Path,
    conversation: &super::store::Conversation,
    answer_index: usize,
) -> Result<(), String> {
    let prompt = conversation.messages[..answer_index.min(conversation.messages.len())]
        .iter()
        .rev()
        .find(|m| matches!(m, rig_core::completion::Message::User { .. }))
        .map(super::context::message_text)
        .filter(|t| !t.trim().is_empty())
        .ok_or("no prompt before that answer")?;
    let wrong = conversation
        .messages
        .get(answer_index)
        .map(super::context::message_text)
        .unwrap_or_default();
    let id = format!(
        "{}-{answer_index}",
        conversation
            .id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(40)
            .collect::<String>()
    );
    add_case(
        config_dir,
        REGRESSIONS,
        &Case {
            id,
            prompt,
            needs_expectation: true,
            note: format!(
                "Rated down on {}. The answer was: {}",
                Utc::now().format("%Y-%m-%d"),
                wrong.chars().take(1_000).collect::<String>()
            ),
            ..Case::default()
        },
    )
}

/// `cortex-desktop eval <suite> [--model ref] [--without kind:name ...]`: runs a suite and prints
/// its score; with `--without`, the comparison with and without. Returns the exit code.
pub fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("eval") {
        return None;
    }
    let Some(suite) = args.get(2).cloned() else {
        eprintln!(
            "cortex: eval <suite> [--model vendor/model] [--without tool:<name>|skill:<name>]"
        );
        return Some(1);
    };
    let model_arg = args
        .iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let without: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--without")
        .map(|w| w[1].clone())
        .collect();
    let without = match Without::parse(&without) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("cortex: {e}");
            return Some(1);
        }
    };
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        eprintln!("cortex: the app's config folder could not be found");
        return Some(1);
    };
    if suite == STARTER {
        if let Err(e) = ensure_starter(&config_dir) {
            eprintln!("cortex: {e}");
            return Some(1);
        }
    }
    let Some(model) = model_arg.or_else(|| super::providers::load(&config_dir).default_model)
    else {
        eprintln!("cortex: no model: pass --model vendor/model, or choose one in the app first");
        return Some(1);
    };
    let host: Arc<dyn Host> = Arc::new(super::headless::HeadlessHost::new(config_dir));
    let report = tauri::async_runtime::block_on(async {
        if without.tools.is_empty() && without.skills.is_empty() {
            run_suite(&host, &suite, &model, &without)
                .await
                .map(|run| serde_json::to_string_pretty(&run).unwrap_or_default())
        } else {
            ablate(&host, &suite, &model, &without)
                .await
                .map(|(_, _, c)| serde_json::to_string_pretty(&c).unwrap_or_default())
        }
    });
    match report {
        Ok(text) => {
            println!("{text}");
            Some(0)
        }
        Err(e) => {
            eprintln!("cortex: {e}");
            Some(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suites_live_outside_the_folder_tools_write() {
        let dir = tempfile::tempdir().unwrap();
        ensure_starter(dir.path()).unwrap();
        assert_eq!(suites(dir.path()), vec![STARTER.to_string()]);
        assert_eq!(cases(dir.path(), STARTER).len(), 3);
        assert!(!evals_dir(dir.path()).starts_with(super::super::store::chat_dir(dir.path())));
        // Running the starter again keeps the user's edits.
        let mut edited = cases(dir.path(), STARTER)[0].clone();
        edited.facts = vec!["1.99".into()];
        add_case(dir.path(), STARTER, &edited).unwrap();
        ensure_starter(dir.path()).unwrap();
        assert!(cases(dir.path(), STARTER)
            .iter()
            .any(|c| c.facts == vec!["1.99".to_string()]));
    }

    #[test]
    fn a_rated_down_answer_becomes_a_waiting_regression_case() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = super::super::store::Conversation::new("chat-1".into(), None, None);
        c.messages.push(rig_core::completion::Message::user(
            "What is the capital of Australia?",
        ));
        c.messages
            .push(rig_core::completion::Message::assistant("Sydney."));
        regression_from_rating(dir.path(), &c, 1).unwrap();
        let saved = cases(dir.path(), REGRESSIONS);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].prompt, "What is the capital of Australia?");
        assert!(saved[0].needs_expectation && saved[0].note.contains("Sydney"));
        // The user says what the right answer contains: the case stops waiting.
        let mut answered = saved[0].clone();
        answered.facts = vec![" Canberra ".into(), "".into()];
        save_case(dir.path(), REGRESSIONS, answered).unwrap();
        let view = &suite_views(dir.path())[0];
        assert_eq!(view.cases[0].facts, vec!["Canberra".to_string()]);
        assert!(!view.cases[0].needs_expectation);
        delete_case(dir.path(), REGRESSIONS, &view.cases[0].id).unwrap();
        assert!(cases(dir.path(), REGRESSIONS).is_empty());
        assert!(delete_case(dir.path(), REGRESSIONS, "../x").is_err());
    }
}
