//! What a turn is asked to do: the message, its images, speed and mode, and where it runs.

use super::*;
use crate::chat::error::ChatError;

/// An image the user attached, as the page read it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
    pub media_type: String,
    /// Base64, without a `data:` prefix.
    pub data: String,
}

/// Images per message, and the size of each once decoded.
pub(super) const MAX_IMAGES: usize = 6;
pub(super) const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// The user's message with its images, checked for type and size.
pub(in crate::chat) fn user_message(
    text: &str,
    images: &[ImageInput],
) -> Result<Message, ChatError> {
    use rig_core::completion::message::{ImageMediaType, MimeType, UserContent};
    if images.is_empty() {
        return Ok(Message::user(text));
    }
    if images.len() > MAX_IMAGES {
        return Err(ChatError::Invalid(format!(
            "Attach at most {MAX_IMAGES} images."
        )));
    }
    let mut content = vec![UserContent::text(if text.is_empty() {
        "What is in this image?"
    } else {
        text
    })];
    for image in images {
        let media = ImageMediaType::from_mime_type(&image.media_type)
            .ok_or_else(|| format!("{} images are not supported.", image.media_type))
            .map_err(ChatError::Invalid)?;
        if image.data.len() / 4 * 3 > MAX_IMAGE_BYTES {
            return Err(ChatError::Invalid("Each image can be at most 8 MB.".into()));
        }
        content.push(UserContent::image_base64(
            image.data.clone(),
            Some(media),
            None,
        ));
    }
    Ok(Message::User { content })
}

/// The composer's speed setting: how much the model reasons (Vibe's thinking levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub enum Speed {
    /// No extended reasoning.
    #[default]
    Fast,
    /// A little reasoning.
    Low,
    /// Reasoning for complex tasks.
    Think,
    /// The most the model offers short of its maximum.
    Deep,
}

impl Speed {
    /// The reasoning effort asked of the model, if any.
    pub(crate) fn effort(self) -> Option<Effort> {
        match self {
            Speed::Fast => None,
            Speed::Low => Some(Effort::Low),
            Speed::Think => Some(Effort::High),
            Speed::Deep => Some(Effort::XHigh),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRequest {
    pub conversation_id: String,
    /// The user's message; empty to regenerate the last reply.
    #[serde(default)]
    pub text: String,
    pub model: String,
    #[serde(default)]
    pub speed: Speed,
    #[serde(default)]
    pub approval: ApprovalMode,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Images sent with the message, for models that can see.
    #[serde(default)]
    pub images: Vec<ImageInput>,
    /// Replace the prompt at this message index with `text` and answer again from there.
    #[serde(default)]
    pub edit_index: Option<usize>,
    /// Keep the conversation out of the history; it is deleted on the next start.
    #[serde(default)]
    pub temporary: bool,
    /// The agent to run as; only read when the conversation is created.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Pick up where a failed or stopped turn left off, with nothing added or removed.
    #[serde(default)]
    pub resume: bool,
    /// Code sessions: the folder to work in. Only read when the conversation is created.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Connector ids to offer; all enabled ones when absent.
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
    /// Why Auto chose this request's model, shown above the answer.
    #[serde(skip)]
    pub route_note: Option<String>,
    /// Built-in tools turned off for this chat in the composer's Tools menu (`generate_image`,
    /// `web_search`, …).
    #[serde(default)]
    pub tools_off: Vec<String>,
    /// Model calls before the turn must answer; [`MAX_STEPS`] when absent.
    #[serde(default)]
    pub max_steps: Option<usize>,
    /// Offer only these tools, by name. For runs with a narrow job, such as a headless run that
    /// only reads.
    #[serde(default)]
    pub only_tools: Option<Vec<String>>,
    /// The whole system prompt, for a run that brings its own brief (a `delegate` worker).
    #[serde(default)]
    pub system: Option<String>,
    /// Skills left out of this turn, to measure what one contributes (an eval's ablation).
    #[serde(default)]
    pub skills_off: Vec<String>,
    /// Saved facts (by id) and knowledge pages (by id) left out of this turn, and whether the
    /// user's standing instructions are: the rest of an eval's ablation.
    #[serde(default)]
    pub facts_off: Vec<String>,
    #[serde(default)]
    pub pages_off: Vec<String>,
    #[serde(default)]
    pub instructions_off: bool,
}
