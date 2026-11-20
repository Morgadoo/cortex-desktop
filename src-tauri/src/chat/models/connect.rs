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
    let first = match attempt(reference.to_string(), request.clone()).await {
        Ok(text) => return Ok(text),
        // A gateway that failed once (a 502, a timeout) usually answers a moment later: one more
        // try, so a background job (a title, a reflection) is not lost to a passing fault.
        Err(e) if transient(&e.to_string()) => {
            log::warn!("{reference} failed ({e}); trying again");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match attempt(reference.to_string(), request.clone()).await {
                Ok(text) => return Ok(text),
                Err(e) => e,
            }
        }
        Err(e) => e,
    };
    match providers::load(config_dir)
        .backup_model
        .filter(|b| b != reference)
    {
        Some(backup) => {
            log::warn!("{reference} failed ({first}); trying the backup model {backup}");
            attempt(backup, request).await.map_err(|_| first)
        }
        None => Err(first),
    }
}

/// Whether a failed call is worth one more try: the provider or its gateway was unavailable for
/// a moment, not the request itself wrong.
fn transient(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "status 500",
        "status 502",
        "status 503",
        "status 504",
        "status 429",
        "did not answer in time",
        "timed out",
        "connection reset",
        "connection closed",
        "error sending request",
    ]
    .iter()
    .any(|sign| error.contains(sign))
}

#[cfg(test)]
mod tests {
    use super::transient;

    #[test]
    fn a_passing_fault_is_tried_again_and_a_wrong_request_is_not() {
        assert!(transient(
            "ProviderResponseError: status 502 Bad Gateway: upstream request failed"
        ));
        assert!(transient("The provider did not answer in time."));
        assert!(transient("status 429 Too Many Requests"));
        assert!(!transient("status 401 Unauthorized: invalid api key"));
        assert!(!transient("status 400 Bad Request: unknown model"));
        assert!(!transient("The model gave no text back."));
    }
}
