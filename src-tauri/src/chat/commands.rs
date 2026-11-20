//! IPC for the chat page. Every command is allowlisted in `permissions/commands.toml`, one permission per file below.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::chat::engine::{self, TurnRequest};
use crate::chat::providers::{self, ModelView, ProviderView};
use crate::chat::store::{self, ConversationSummary, ConversationView, Instructions, Project};
use crate::chat::ChatState;

// The commands, by area; each is re-exported here, where the app's handler list names them.
mod agents_commands;
pub use agents_commands::*;
mod app_commands;
pub use app_commands::*;
mod canvases_commands;
pub use canvases_commands::*;
mod code_commands;
pub use code_commands::*;
mod conversations_commands;
pub use conversations_commands::*;
mod exports_commands;
pub use exports_commands::*;
mod knowledge_commands;
pub use knowledge_commands::*;
mod libraries_commands;
pub use libraries_commands::*;
mod providers_commands;
pub use providers_commands::*;
mod recording_commands;
pub use recording_commands::*;
mod search_commands;
pub use search_commands::*;
mod settings_commands;
pub use settings_commands::*;
mod sharing_commands;
pub use sharing_commands::*;
mod tasks_commands;
pub use tasks_commands::*;
mod tools_commands;
pub use tools_commands::*;
mod voice_commands;
pub use voice_commands::*;
mod writing_commands;
pub use writing_commands::*;

fn config_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    crate::settings::get_config_dir(app)
}

fn secrets() -> Result<crate::tools::auth::Secrets, String> {
    crate::chat::secrets::local().map_err(String::from)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProvider {
    pub vendor: String,
    pub enabled: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub custom_models: Vec<String>,
    /// A new key; absent keeps the saved one.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_key: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub models: Vec<ModelView>,
    pub default_model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    feature = "typescript",
    derive(ts_rs::TS),
    ts(export, rename = "Conversation")
)]
pub struct OpenConversation {
    #[serde(flatten)]
    pub view: ConversationView,
    pub running: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConversation {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    /// `Some(None)` moves it out of its project.
    #[serde(default, deserialize_with = "serde_with_none::deserialize")]
    pub project_id: Option<Option<String>>,
    #[serde(default)]
    pub unread: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// `(turn index, -1 | 0 | 1)`.
    #[serde(default)]
    pub rating: Option<(usize, i8)>,
}

/// `Option<Option<T>>` that tells a missing field from an explicit `null`.
mod serde_with_none {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Ok(Some(Option::deserialize(deserializer)?))
    }
}

/// An artifact with its lifecycle score (`cortex_kernel::lifecycle::score`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredArtifact {
    #[serde(flatten)]
    artifact: crate::chat::traces::Artifact,
    score: f64,
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpeechOut {
    Audio {
        data: String,
        mime: String,
    },
    Browser {
        text: String,
        rate: f32,
        voice: String,
    },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioView {
    pub settings: crate::chat::audio::AudioSettings,
    /// What the `provider` engines resolve to now.
    pub provider_speech_model: Option<String>,
    pub provider_voice_model: Option<String>,
}

fn follow_up_prompt(question: &str, answer: &str) -> String {
    let cut = |t: &str, n: usize| crate::support::text::clip(t, n);
    format!(
        "Suggest three short follow-up messages the user might send next in this conversation, \
         written as the user, in the user's language. Each under 12 words, each a different \
         direction (go deeper, apply it, an alternative). One per line, no numbering, no quotes.\n\n\
         User: {}\n\nAssistant: {}",
        cut(question, 2_000),
        cut(answer, 4_000)
    )
}

/// Lines of the reply, without list marks or quotes, at most three.
fn parse_follow_ups(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(|c: char| {
                    c.is_ascii_digit() || matches!(c, '-' | '*' | '•' | '.' | ')')
                })
                .trim()
        })
        .map(|l| {
            l.trim_matches(|c| matches!(c, '"' | '“' | '”'))
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty() && l.chars().count() <= 120 && !l.ends_with(':'))
        .take(3)
        .collect()
}

fn sanitize_file_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim();
    if name.is_empty() {
        "conversation".into()
    } else {
        crate::support::text::clip(name, 80)
    }
}

/// Sync between devices, as this device has it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    #[serde(flatten)]
    settings: crate::chat::sync::Settings,
    has_passphrase: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export, rename = "Task"))]
pub struct TaskView {
    #[serde(flatten)]
    pub task: crate::chat::tasks::Task,
    pub schedule: String,
}

fn task_views(tasks: Vec<crate::chat::tasks::Task>) -> Vec<TaskView> {
    tasks
        .into_iter()
        .map(|task| TaskView {
            schedule: crate::chat::tasks::describe(&task.frequency),
            task,
        })
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddResult {
    pub library: crate::chat::libraries::Library,
    pub errors: Vec<String>,
}

/// The text in an image, read by the default model: OCR with any model that can see.
async fn ocr(
    config_dir: &std::path::Path,
    path: &std::path::Path,
    media_type: &str,
) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = crate::chat::at_rest::read(path).map_err(|e| e.to_string())?;
    let file = providers::load(config_dir);
    let reference = match file.default_model.clone() {
        Some(r) => r,
        None => providers::models(&file)
            .await
            .first()
            .map(|m| m.reference.clone())
            .ok_or("Set up a model first.")?,
    };
    let image = engine::ImageInput {
        media_type: media_type.into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    };
    let message = engine::user_message(OCR_PROMPT, &[image])?;
    let request = rig_core::completion::CompletionRequest::new(message).max_tokens(8_000u64);
    let text = engine::ask_text(config_dir, &reference, request, 180)
        .await
        .map_err(|e| {
            format!("{reference} could not read it ({e}); it needs a model that can see images.")
        })?;
    if text.trim().eq_ignore_ascii_case("NO TEXT") {
        return Err("no text was found in the image.".into());
    }
    Ok(text)
}

const OCR_PROMPT: &str = "Transcribe all the text in this image exactly, in reading order, keeping headings, lists and \
    table rows (as Markdown). Describe charts, photos and diagrams in one short line each, in square brackets. \
    Output only the transcription. If there is no text at all, reply NO TEXT.";

/// Mirrors a folder the user picks into a library.
#[tauri::command]
pub async fn chat_add_library_folder(
    app: AppHandle,
    id: String,
) -> Result<Option<crate::chat::libraries::SyncReport>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) =
        crate::support::dialogs::pick_folder(app.dialog().file().set_title("Mirror a folder"))
            .await?
    else {
        return Ok(None);
    };
    let folder = path.into_path().map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::libraries::add_folder(&dir, &id, &folder))
        .await
        .map_err(|e| e.to_string())?
        .map(Some)
        .map_err(String::from)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryConnector {
    id: String,
    name: String,
    mirrored: bool,
}

fn emoji_root(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager as _;
    app.path().app_local_data_dir().map_err(|e| e.to_string())
}

/// A workspace file's real path, checked to be inside that chat's workspace.
fn workspace_file(
    app: &AppHandle,
    conversation_id: &str,
    path: &str,
) -> Result<std::path::PathBuf, String> {
    let ws = crate::chat::sandbox::workspace(&config_dir(app)?, conversation_id)?;
    let (Ok(root), Ok(real)) = (ws.canonicalize(), ws.join(path).canonicalize()) else {
        return Err("No such file.".into());
    };
    if !real.starts_with(&root) {
        return Err("No such file.".into());
    }
    Ok(real)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiServerView {
    settings: crate::chat::api_server::ApiServerSettings,
    status: crate::chat::api_server::Status,
    /// Shown so the user can paste it into other tools; only once the server was turned on.
    token: Option<String>,
    /// The website widget's key, while the widget is on.
    widget_token: Option<String>,
}

async fn api_server_view(app: &AppHandle) -> Result<ApiServerView, String> {
    let settings = crate::chat::api_server::load(&config_dir(app)?);
    let status = app.state::<crate::chat::api_server::ApiServer>().status();
    let token = if settings.enabled {
        Some(crate::chat::api_server::token(false).await?)
    } else {
        None
    };
    let widget_token = if settings.enabled && settings.widget.enabled {
        Some(crate::chat::api_server::widget_token(false).await?)
    } else {
        None
    };
    Ok(ApiServerView {
        settings,
        status,
        token,
        widget_token,
    })
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    if !from.exists() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(from)
        .into_iter()
        .filter_map(Result::ok)
    {
        let rel = entry.path().strip_prefix(from).map_err(|e| e.to_string())?;
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            // Exports leave the app's folder in the clear, even when it is encrypted.
            let data = crate::chat::at_rest::read(entry.path()).map_err(|e| e.to_string())?;
            std::fs::write(&dest, data).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn conversation_markdown(conversation: &store::Conversation) -> String {
    let title = if conversation.title.is_empty() {
        engine::fallback_title(conversation)
    } else {
        conversation.title.clone()
    };
    let mut text = format!("# {title}\n\n");
    for turn in store::turns(conversation) {
        text.push_str(if turn.role == "user" {
            "## You\n\n"
        } else {
            "## Assistant\n\n"
        });
        for part in turn.parts {
            match part {
                store::Part::Text { text: t } => text.push_str(&format!("{t}\n\n")),
                store::Part::Tool { name, .. } => text.push_str(&format!("_Used {name}_\n\n")),
                store::Part::Image { .. } => text.push_str("_[image]_\n\n"),
                store::Part::Reasoning { .. } => {}
                store::Part::Notice { text: t } => text.push_str(&format!("> {t}\n\n")),
            }
        }
    }
    text
}

/// One canvas in a list: enough to pick it, without its versions.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasEntry {
    id: String,
    title: String,
    kind: String,
    conversation_id: String,
}

/// The tool names an agent can be given, matching the editor's choices.
const TOOL_CHOICES: &[&str] = &[
    "web_search",
    "browser",
    "filesystem",
    "bash",
    "delegate",
    "update_plan",
    "canvas",
    "schedule_task",
    "use_skill",
    "remember",
    "run_code",
    "calendar_events",
    "find_contact",
    "search_chats",
    "knowledge_search",
    "notify",
    "generate_image",
    "generate_video",
    "make_animation",
    "browser_view",
    "browser_act",
    "message_agent",
];

/// Tokens and cost across all conversations this calendar month, for the usage gauge.
#[tauri::command]
pub fn chat_usage_month(app: AppHandle) -> Result<crate::chat::agents::Spend, String> {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let mut total = crate::chat::agents::Spend::default();
    for c in store::list_conversations(&config_dir(&app)?) {
        if c.updated_at.year() == now.year() && c.updated_at.month() == now.month() {
            total.runs += 1;
            total.tokens += c.tokens;
            total.cost += c.cost;
        }
    }
    Ok(total)
}

/// This month's usage by model and by agent (LobeHub's stats), largest first.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLine {
    name: String,
    chats: u64,
    tokens: u64,
    cost: f64,
}

/// Manages chat state at startup.
pub(crate) fn setup(app: &tauri::App) {
    app.manage(ChatState::default());
    if let Ok(dir) = config_dir(app.handle()) {
        crate::chat::store::purge_temporary(&dir);
        // Off the startup path: copying state files and pruning old backups can touch many files.
        std::thread::spawn(move || crate::chat::backup::daily(&dir));
    }
    crate::chat::tasks::start(app.handle().clone());
    app.manage(crate::chat::api_server::ApiServer::default());
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        crate::chat::api_server::apply(&handle).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_ups_are_clean_lines() {
        let got = parse_follow_ups("1. How do I start?\n- \"Show an example\"\n\nHere are some:\nWhat about costs?\nA fourth one");
        assert_eq!(
            got,
            vec!["How do I start?", "Show an example", "What about costs?"]
        );
        assert!(follow_up_prompt("q", "a").contains("three short follow-up"));
    }

    #[test]
    fn update_distinguishes_null_from_missing_project() {
        let missing: UpdateConversation = serde_json::from_str(r#"{"id":"a"}"#).unwrap();
        assert_eq!(missing.project_id, None);
        let cleared: UpdateConversation =
            serde_json::from_str(r#"{"id":"a","projectId":null}"#).unwrap();
        assert_eq!(cleared.project_id, Some(None));
        let set: UpdateConversation =
            serde_json::from_str(r#"{"id":"a","projectId":"p"}"#).unwrap();
        assert_eq!(set.project_id, Some(Some("p".into())));
    }

    #[test]
    fn export_names_are_safe() {
        assert_eq!(sanitize_file_name("a/b: c?"), "a_b_ c_");
        assert_eq!(sanitize_file_name("   "), "conversation");
    }
}
