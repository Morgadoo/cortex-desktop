//! Writing help: translating answers and the interface, the interface's translations and their coverage, and rewriting text in place.

use super::*;

/// Translates the interface text still missing for `code` with `model`.
#[tauri::command]
pub async fn chat_translate_ui(
    app: AppHandle,
    model: String,
    code: String,
) -> Result<crate::chat::ui_language::Coverage, String> {
    crate::chat::ui_language::translate(&config_dir(&app)?, &model, &code)
        .await
        .map_err(String::from)
}

/// An answer translated into `language`, Markdown kept; shown beside the original, not stored.
#[tauri::command]
pub async fn chat_translate(
    app: AppHandle,
    model: String,
    text: String,
    language: String,
) -> Result<String, String> {
    let language: String = crate::support::text::clip(language.trim(), 40);
    if language.is_empty() || text.trim().is_empty() {
        return Err("Nothing to translate.".into());
    }
    let prompt = format!(
        "Translate the text below into {language}. Keep its Markdown, code blocks, links and numbers exactly; \
         translate only the prose. Reply with the translation alone.\n\n{}",
        crate::support::text::clip(&text, 30_000)
    );
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .max_tokens(8_000u64);
    engine::ask_text(&config_dir(&app)?, &model, request, 120)
        .await
        .map_err(String::from)
}

/// Rewrites `text` as asked (shorter, clearer, fixed grammar, …) for editing in place. `context`
/// is the text around it, so a rewritten passage still fits.
#[tauri::command]
pub async fn chat_improve_text(
    app: AppHandle,
    model: String,
    text: String,
    instruction: String,
    context: Option<String>,
) -> Result<String, String> {
    let instruction: String = crate::support::text::clip(instruction.trim(), 400);
    if text.trim().is_empty() {
        return Err("Nothing to improve.".into());
    }
    let instruction = if instruction.is_empty() {
        "Improve the writing: clearer and tighter, same meaning.".to_string()
    } else {
        instruction
    };
    let context = context
        .filter(|c| !c.trim().is_empty())
        .map(|c| format!("\n\nThe text around it, for reference only (do not repeat it):\n<context>\n{}\n</context>", crate::support::text::clip(&c, 12_000)))
        .unwrap_or_default();
    let prompt = format!(
        "Rewrite the text below. Instruction: {instruction}\n\
         Keep its Markdown, links, code and facts; do not add facts. Reply with the rewritten text alone, \
         with no preamble or quotes.{context}\n\n<text>\n{}\n</text>",
        crate::support::text::clip(&text, 30_000)
    );
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .max_tokens(8_000u64);
    let out = engine::ask_text(&config_dir(&app)?, &model, request, 120).await?;
    let out = out
        .trim()
        .trim_start_matches("<text>")
        .trim_end_matches("</text>")
        .trim();
    if out.is_empty() {
        return Err("The model gave no text back.".into());
    }
    Ok(out.to_string())
}

/// Every translation of interface text known for `code`.
#[tauri::command]
pub fn chat_ui_translations(
    app: AppHandle,
    code: String,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    Ok(crate::chat::ui_language::translations(
        &config_dir(&app)?,
        &code,
    ))
}

/// How much of the interface `code` covers.
#[tauri::command]
pub fn chat_ui_coverage(
    app: AppHandle,
    code: String,
) -> Result<crate::chat::ui_language::Coverage, String> {
    Ok(crate::chat::ui_language::coverage(
        &config_dir(&app)?,
        &code,
    ))
}
