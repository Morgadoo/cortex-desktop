//! Connecting to a model by its reference, and one-off requests for text with the backup model
//! as a fallback: what the engine, the app's own errands and the recorder all use.

use crate::chat::error::ChatError;
use rig_core::completion::CompletionRequest;
use rig_core::operation::Completion;
use rig_core::DynModel;

use crate::chat::providers;

pub async fn connect_model(
    config_dir: &std::path::Path,
    reference: &str,
) -> Result<DynModel<Completion>, ChatError> {
    // The app's own errands on Auto use its fast model.
    let routed;
    let reference = if reference == crate::chat::router::AUTO {
        routed = crate::chat::router::models(&providers::load(config_dir))
            .map(|(fast, _)| fast)
            .ok_or_else(|| {
                ChatError::Invalid("Choose Auto's fast and strong models first.".into())
            })?;
        routed.as_str()
    } else {
        reference
    };
    if reference.starts_with("local/") {
        return crate::chat::local_models::connect(reference).await;
    }
    let (vendor, _) = providers::split_reference(reference)
        .ok_or_else(|| format!("`{reference}` is not a model reference"))
        .map_err(ChatError::Invalid)?;
    let file = providers::load(config_dir);
    let settings = file.providers.get(vendor).cloned().unwrap_or_default();
    let key = if settings.has_key {
        crate::chat::providers::api_key(vendor).await?
    } else {
        None
    };
    providers::connect(reference, &settings, key)
}

/// One request for a text, for the app's own errands (titles, follow-ups, translations,
/// rewrites): when the model fails or does not answer in time, the backup model set under
/// Providers is tried once, as chats do. The error is the first model's, in plain words.
pub async fn ask_text(
    config_dir: &std::path::Path,
    reference: &str,
    request: CompletionRequest,
    secs: u64,
) -> Result<String, ChatError> {
    let attempt = |reference: String, request: CompletionRequest| async move {
        let model = connect_model(config_dir, &reference).await?;
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(secs), model.call(request))
                .await
                .map_err(|_| "The provider did not answer in time.".to_string())
                .map_err(ChatError::Invalid)?
                .map_err(|e| cortex_kernel::context::short_error(&e.to_string()))
                .map_err(ChatError::Invalid)?;
        let text = response.text();
        // A thinking model can spend a small budget on thought and say nothing.
        if text.trim().is_empty() {
            return Err(ChatError::Invalid("The model gave no text back.".into()));
        }
        Ok::<String, String>(text).map_err(ChatError::Invalid)
    };
    match attempt(reference.to_string(), request.clone()).await {
        Ok(text) => Ok(text),
        Err(first) => match providers::load(config_dir)
            .backup_model
            .filter(|b| b != reference)
        {
            Some(backup) => {
                log::warn!("{reference} failed ({first}); trying the backup model {backup}");
                attempt(backup, request).await.map_err(|_| first)
            }
            None => Err(first),
        },
    }
}
