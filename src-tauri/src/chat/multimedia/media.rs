//! Images and speech: the `generate_image` tool and voice input, through rig's OpenAI-shaped
//! image and transcription models.
//!
//! Each uses a provider the user already set up, with its key and base URL. The model is the
//! one chosen in settings, or a sensible default for the first provider that offers one.

use crate::chat::error::ChatError;
use std::path::Path;

use rig_core::audio_generation::AudioGenerationRequestBuilder;
use rig_core::image_generation::ImageGenerationRequestBuilder;
use rig_core::providers::openai::wire::{self as dialects, Dialect};
use rig_core::providers::openai::{OpenAI, OpenAIConfig};
use rig_core::providers::xai;
use rig_core::transcription::TranscriptionRequestBuilder;
use serde::Serialize;
use serde_json::{json, Value};

use crate::chat::providers::{self, ProviderSettings, ProvidersFile};
use cortex_kernel::tool::{ChatTool, Outcome};

pub const GENERATE_IMAGE: &str = "generate_image";

/// Providers whose image endpoint rig speaks, with the model used when none is chosen.
const IMAGE_DEFAULTS: &[(&str, &str)] = &[
    ("openai", "gpt-image-1"),
    ("xai", "grok-imagine-image"),
    ("together", "black-forest-labs/FLUX.1-schnell"),
];
/// Providers that read text aloud, with the model used when none is chosen.
const VOICE_DEFAULTS: &[(&str, &str)] = &[("openai", "gpt-4o-mini-tts"), ("xai", "grok-tts")];

/// Providers with a speech-to-text endpoint, with the model used when none is chosen.
const SPEECH_DEFAULTS: &[(&str, &str)] = &[
    ("openai", "gpt-4o-mini-transcribe"),
    ("groq", "whisper-large-v3-turbo"),
];

/// What settings shows: the models in use, and the providers that could serve each.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaModels {
    pub image_model: Option<String>,
    pub speech_model: Option<String>,
    pub voice_model: Option<String>,
    pub image_vendors: Vec<String>,
    pub speech_vendors: Vec<String>,
    pub voice_vendors: Vec<String>,
    pub image_server: Option<String>,
    pub video_model: Option<String>,
    pub video_vendors: Vec<String>,
}

fn dialect(vendor: &str) -> Option<&'static Dialect> {
    Some(match vendor {
        "openai" => &dialects::OPENAI,
        "xai" => &xai::DIALECT,
        "groq" => &dialects::GROQ,
        "together" => &dialects::TOGETHER,
        _ => return None,
    })
}

/// Whether `vendor` is enabled and has a key (saved, or in the environment).
pub(crate) fn usable(file: &ProvidersFile, vendor: &str) -> bool {
    file.providers
        .get(vendor)
        .is_some_and(|s| s.enabled && (s.has_key || providers::env_key(vendor).is_some()))
}

fn pick(
    file: &ProvidersFile,
    chosen: Option<&String>,
    defaults: &[(&str, &str)],
) -> Option<String> {
    if let Some(chosen) = chosen.filter(|c| !c.trim().is_empty()) {
        let vendor = providers::split_reference(chosen)?.0;
        return (usable(file, vendor) && dialect(vendor).is_some()).then(|| chosen.clone());
    }
    defaults
        .iter()
        .find(|(vendor, _)| usable(file, vendor))
        .map(|(vendor, model)| format!("{vendor}/{model}"))
}

pub(in crate::chat::multimedia) fn image_model(file: &ProvidersFile) -> Option<String> {
    // A local image server needs no key: chosen, or the only image maker there is.
    let local = file.image_server.as_ref().filter(|u| !u.trim().is_empty());
    if let Some(chosen) = file
        .image_model
        .as_ref()
        .filter(|m| m.starts_with(SDWEBUI_PREFIX))
    {
        return local.map(|_| chosen.clone());
    }
    pick(file, file.image_model.as_ref(), IMAGE_DEFAULTS)
        .or_else(|| local.map(|_| format!("{SDWEBUI_PREFIX}default")))
}

// ---------------------------------------------------------------------------------------------
// A local image server (Open WebUI's and LibreChat's local image generation; on a Mac, Draw
// Things makes images on the device): the AUTOMATIC1111 API that Stable Diffusion WebUI,
// Forge, SD.Next and Draw Things serve. `txt2img` makes an image; `img2img` starts from one.

pub const SDWEBUI_PREFIX: &str = "sdwebui/";

/// The request body for the AUTOMATIC1111 API.
pub(in crate::chat::multimedia) fn sdwebui_body(
    prompt: &str,
    width: u32,
    height: u32,
    seed: Option<u64>,
    checkpoint: Option<&str>,
    init_image: Option<&str>,
) -> Value {
    let mut body = json!({
        "prompt": prompt,
        "negative_prompt": "blurry, low quality, watermark, text artifacts",
        "width": width,
        "height": height,
        "steps": 25,
        "cfg_scale": 7,
        "seed": seed.map_or(-1i64, |s| s as i64),
        "batch_size": 1,
    });
    if let Some(c) = checkpoint.filter(|c| !c.is_empty() && *c != "default") {
        body["override_settings"] = json!({ "sd_model_checkpoint": c });
    }
    if let Some(img) = init_image {
        body["init_images"] = json!([img]);
        body["denoising_strength"] = json!(0.6);
    }
    body
}

/// Makes one image on the local server; returns its bytes.
pub(in crate::chat::multimedia) async fn sdwebui_image(
    server: &str,
    model: &str,
    prompt: &str,
    width: u32,
    height: u32,
    seed: Option<u64>,
    init_image: Option<&str>,
) -> Result<Vec<u8>, ChatError> {
    use base64::Engine as _;
    let base = server.trim().trim_end_matches('/');
    let endpoint = if init_image.is_some() {
        "img2img"
    } else {
        "txt2img"
    };
    let checkpoint = model.strip_prefix(SDWEBUI_PREFIX);
    let client = crate::support::http::client(std::time::Duration::from_secs(600))
        .map_err(ChatError::Invalid)?;
    let response = client
        .post(format!("{base}/sdapi/v1/{endpoint}"))
        .json(&sdwebui_body(
            prompt, width, height, seed, checkpoint, init_image,
        ))
        .send()
        .await
        .map_err(|e| {
            ChatError::Invalid(format!(
                "The local image server did not answer at {base}: {e}"
            ))
        })?;
    if !response.status().is_success() {
        return Err(ChatError::Invalid(format!(
            "The local image server answered {}.",
            response.status()
        )));
    }
    let body: Value = response.json().await.map_err(|e| {
        ChatError::Invalid(format!(
            "The local image server sent something unexpected: {e}"
        ))
    })?;
    let b64 = body["images"]
        .get(0)
        .and_then(Value::as_str)
        .ok_or_else(|| ChatError::Invalid("The local image server sent back no image.".into()))?;
    // Some servers prefix a data URL.
    let b64 = b64
        .split_once(",")
        .filter(|(head, _)| head.starts_with("data:"))
        .map_or(b64, |(_, rest)| rest);
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(ChatError::internal)
}

fn image_server(config_dir: &Path) -> Option<String> {
    providers::load(config_dir)
        .image_server
        .filter(|u| !u.trim().is_empty())
}

/// The base64 of a reference image, for img2img.
pub(crate) fn image_b64(image: &rig_core::completion::message::Image) -> Option<String> {
    match &image.data {
        rig_core::completion::message::DocumentSourceKind::Base64(b) => Some(b.clone()),
        _ => None,
    }
}

pub fn speech_model(file: &ProvidersFile) -> Option<String> {
    pick(file, file.speech_model.as_ref(), SPEECH_DEFAULTS)
}

pub fn voice_model(file: &ProvidersFile) -> Option<String> {
    pick(file, file.voice_model.as_ref(), VOICE_DEFAULTS)
}

pub fn models(config_dir: &Path) -> MediaModels {
    let file = providers::load(config_dir);
    let vendors = |defaults: &[(&str, &str)]| {
        defaults
            .iter()
            .filter(|(v, _)| usable(&file, v))
            .map(|(v, _)| v.to_string())
            .collect()
    };
    MediaModels {
        image_model: image_model(&file),
        speech_model: speech_model(&file),
        voice_model: voice_model(&file),
        image_vendors: vendors(IMAGE_DEFAULTS),
        speech_vendors: vendors(SPEECH_DEFAULTS),
        voice_vendors: vendors(VOICE_DEFAULTS),
        image_server: file.image_server.clone(),
        video_model: crate::chat::videogen::video_model(&file),
        video_vendors: crate::chat::videogen::vendors(&file),
    }
}

/// An OpenAI-shaped client for `vendor`, with the user's key and base URL.
fn client(
    vendor: &str,
    settings: &ProviderSettings,
    key: Option<String>,
) -> Result<OpenAI, ChatError> {
    let dialect = dialect(vendor)
        .ok_or_else(|| format!("{vendor} has no image or speech endpoint here."))
        .map_err(ChatError::Invalid)?;
    let key = key
        .filter(|k| !k.is_empty())
        .or_else(|| providers::env_key(vendor).and_then(|name| std::env::var(name).ok()))
        .ok_or_else(|| format!("Add an API key for {vendor} in Settings → Providers."))
        .map_err(ChatError::Invalid)?;
    let mut config = OpenAIConfig::with_key(dialect, key);
    if let Some(base_url) = settings.base_url.as_ref().filter(|u| !u.trim().is_empty()) {
        config = config.with_base_url(providers::normalize_base_url(vendor, base_url));
    }
    Ok(config.client())
}

pub(crate) async fn connect(
    config_dir: &Path,
    reference: &str,
) -> Result<(OpenAI, String), ChatError> {
    let (vendor, model) = providers::split_reference(reference)
        .ok_or_else(|| format!("`{reference}` is not vendor/model"))
        .map_err(ChatError::Invalid)?;
    let settings = providers::load(config_dir)
        .providers
        .get(vendor)
        .cloned()
        .unwrap_or_default();
    let key = if settings.has_key {
        crate::chat::providers::api_key(vendor).await?
    } else {
        None
    };
    Ok((client(vendor, &settings, key)?, model.to_string()))
}

/// The `generate_image` tool, when a provider can make images.
pub fn definition(config_dir: &Path) -> Option<ChatTool> {
    let model = image_model(&providers::load(config_dir))?;
    Some(ChatTool {
        name: GENERATE_IMAGE.into(),
        description: format!(
            "Create an image from a description: illustrations, photos, logos, diagrams with a \
             look. Write a detailed prompt (subject, style, composition, colors, lighting). The \
             image is shown to the user and saved in this conversation's workspace, where \
             run_code can use it. To edit an image or make a variation, pass it as `reference`. \
             Uses {model}."
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "A detailed description of the image, or for an edit, what to change." },
                "shape": { "type": "string", "enum": ["square", "landscape", "portrait"], "default": "square" },
                "reference": { "type": "string", "description": "To edit or vary an image: `attached` for the latest image the user attached, or the `file` an earlier generate_image call returned (images/...)." },
                "count": { "type": "integer", "minimum": 1, "maximum": 4, "description": "How many images (variations). Default 1." },
                "quality": { "type": "string", "enum": ["low", "medium", "high", "auto"] },
                "seed": { "type": "integer", "description": "Only when the user gives one, to make an image again." }
            },
            "required": ["prompt"]
        }),
        connector_id: None,
        tool_name: GENERATE_IMAGE.into(),
        read_only: true,
    })
}

fn extension(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, ..] => "jpg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        [b'G', b'I', b'F', ..] => "gif",
        _ => "png",
    }
}

/// Generates images (one to four) and saves them in the conversation's workspace.
pub async fn generate(config_dir: &Path, conversation_id: &str, args: &Value) -> Outcome {
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let request = StudioRequest {
        prompt: s("prompt").unwrap_or_default(),
        model: None,
        shape: s("shape").unwrap_or_default(),
        quality: s("quality"),
        count: args
            .get("count")
            .and_then(Value::as_u64)
            .map_or(1, |c| c.clamp(1, 4) as u32),
        seed: args.get("seed").and_then(Value::as_u64),
        reference: s("reference"),
    };
    let model = image_model(&providers::load(config_dir)).unwrap_or_default();
    match make_images(config_dir, conversation_id, request).await {
        Ok((files, errors)) => Outcome {
            text: json!({ "file": files[0], "files": files, "model": model, "failed": errors })
                .to_string(),
            is_error: false,
        },
        Err(e) => Outcome::err(e),
    }
}

/// The image an edit starts from: the user's latest attachment, or an image in the workspace.
pub(crate) fn reference_image(
    config_dir: &Path,
    conversation_id: &str,
    reference: &str,
) -> Result<rig_core::completion::message::Image, ChatError> {
    use base64::Engine as _;
    use rig_core::completion::message::{
        DocumentSourceKind, Image, ImageMediaType, Message, UserContent,
    };
    let (data, media_type) = if reference == "attached" {
        let conversation = crate::chat::store::load_conversation(config_dir, conversation_id)?;
        conversation
            .messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { content } => content.iter().find_map(|c| match c {
                    UserContent::Image(image) => match &image.data {
                        DocumentSourceKind::Base64(b64) => {
                            Some((b64.clone(), image.media_type.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                }),
                _ => None,
            })
            .ok_or_else(|| {
                ChatError::Invalid("No image was attached in this conversation.".into())
            })?
    } else {
        // Only files inside this conversation's workspace.
        let root = crate::chat::sandbox::workspace(config_dir, conversation_id)
            .map_err(ChatError::Invalid)?;
        let canonical = root
            .join(reference)
            .canonicalize()
            .map_err(|_| ChatError::Invalid(format!("There is no image at {reference}.")))?;
        if !canonical.starts_with(root.canonicalize().map_err(ChatError::internal)?) {
            return Err(ChatError::Invalid(
                "The reference must be an image in this conversation's workspace.".into(),
            ));
        }
        let bytes = crate::chat::at_rest::read(&canonical).map_err(ChatError::internal)?;
        let media = ImageMediaType::sniff(&bytes);
        (
            base64::engine::general_purpose::STANDARD.encode(bytes),
            media,
        )
    };
    Ok(Image {
        data: DocumentSourceKind::Base64(data),
        media_type,
        detail: None,
        native: None,
    })
}

/// Characters read aloud at most; long answers are cut at a sentence.
const SPEAK_CHARS: usize = 4_000;

/// The prose of a Markdown answer, as it should sound: no code, links read as their text.
pub fn speakable(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            if in_code {
                out.push_str("(code omitted)\n");
            }
            continue;
        }
        if in_code
            || trimmed.starts_with('|')
                && trimmed.matches('|').count() > 2
                && trimmed.contains("---")
        {
            continue;
        }
        let line = trimmed
            .trim_start_matches(['#', '>', '-', '*', '+'])
            .trim_start();
        out.push_str(line);
        out.push('\n');
    }
    // [text](url) → text; drop emphasis marks and inline code ticks.
    let links = regex::Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("valid regex");
    let mut text = links.replace_all(&out, "$1").replace(['*', '_', '`'], "");
    if text.chars().count() > SPEAK_CHARS {
        let cut: String = crate::support::text::clip(&text, SPEAK_CHARS);
        let end = cut.rfind(['.', '!', '?']).map_or(cut.len(), |i| i + 1);
        text = cut[..end].to_string();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads `text` aloud with the chosen voice model; returns the audio and its media type.
pub(in crate::chat::multimedia) async fn speak(
    config_dir: &Path,
    text: &str,
) -> Result<(Vec<u8>, String), ChatError> {
    let reference = voice_model(&providers::load(config_dir)).ok_or_else(|| {
        ChatError::Invalid(
            "Read aloud needs a provider with text-to-speech (OpenAI or xAI) in Settings → Providers."
                .into(),
        )
    })?;
    let (vendor, _) = providers::split_reference(&reference).unwrap_or_default();
    let (client, model) = connect(config_dir, &reference).await?;
    let text = speakable(text);
    if text.is_empty() {
        return Err(ChatError::Invalid("There is nothing to read.".into()));
    }
    let voice = if vendor == "xai" { "eve" } else { "alloy" };
    let request = AudioGenerationRequestBuilder::new(text, voice).build();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        client.audio_generation(model).call(request),
    )
    .await
    .map_err(|_| "Reading aloud took too long.".to_string())
    .map_err(ChatError::Invalid)?
    .map_err(ChatError::internal)?;
    let mime = match response.audio.as_slice() {
        [b'R', b'I', b'F', b'F', ..] => "audio/wav",
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        _ => "audio/mpeg",
    };
    Ok((response.audio, mime.into()))
}

/// Turns recorded speech into text with the chosen speech model.
pub(in crate::chat::multimedia) async fn transcribe(
    config_dir: &Path,
    audio: Vec<u8>,
    file_name: &str,
) -> Result<String, ChatError> {
    let reference = speech_model(&providers::load(config_dir))
        .ok_or_else(|| ChatError::Invalid("Voice input needs a provider with speech-to-text (OpenAI or Groq) in Settings → Providers.".into()))?;
    let (client, model) = connect(config_dir, &reference).await?;
    let request = TranscriptionRequestBuilder::new(audio)
        .filename(Some(file_name.to_string()))
        .build();
    let response = client
        .transcription(model)
        .call(request)
        .await
        .map_err(ChatError::internal)?;
    Ok(response.text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(vendors: &[&str]) -> ProvidersFile {
        let mut f = ProvidersFile::default();
        for v in vendors {
            f.providers.insert(
                v.to_string(),
                ProviderSettings {
                    enabled: true,
                    has_key: true,
                    ..Default::default()
                },
            );
        }
        f
    }

    #[test]
    fn picks_a_model_from_the_providers_set_up() {
        assert_eq!(
            image_model(&file(&["xai"])).as_deref(),
            Some("xai/grok-imagine-image")
        );
        assert_eq!(
            image_model(&file(&["openai", "xai"])).as_deref(),
            Some("openai/gpt-image-1")
        );
        assert_eq!(image_model(&file(&["anthropic"])), None);
        assert_eq!(
            speech_model(&file(&["groq"])).as_deref(),
            Some("groq/whisper-large-v3-turbo")
        );
        let mut chosen = file(&["openai", "xai"]);
        chosen.image_model = Some("xai/grok-imagine-image-pro".into());
        assert_eq!(
            image_model(&chosen).as_deref(),
            Some("xai/grok-imagine-image-pro")
        );
        // A choice whose provider was turned off falls back to nothing rather than failing later.
        chosen.providers.get_mut("xai").unwrap().enabled = false;
        assert_eq!(image_model(&chosen), None);
    }

    /// Answers one HTTP request with `body`, returning what was asked.
    fn serve_once(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn transcribes_through_an_openai_shaped_endpoint() {
        let (url, server) = serve_once(r#"{"text":"  hello there  "}"#);
        let settings = ProviderSettings {
            enabled: true,
            has_key: true,
            base_url: Some(url),
            custom_models: vec![],
        };
        let client = client("openai", &settings, Some("test-key".into())).unwrap();
        let request = TranscriptionRequestBuilder::new(b"RIFFfake".to_vec())
            .filename(Some("speech.webm".into()))
            .build();
        let response = client
            .transcription("whisper-1")
            .call(request)
            .await
            .unwrap();
        assert_eq!(response.text.trim(), "hello there");
        let asked = server.join().unwrap();
        assert!(
            asked.starts_with("POST /v1/audio/transcriptions"),
            "{asked}"
        );
        assert!(asked.contains("Bearer test-key"));
        assert!(asked.contains("whisper-1"));
    }

    #[test]
    fn reads_prose_not_markup() {
        let text = speakable("# Title\n\nSee **[the docs](https://x.y)** for `code`.\n\n```rust\nfn main() {}\n```\n- one\n- two");
        assert_eq!(text, "Title See the docs for code. (code omitted) one two");
    }

    #[test]
    fn knows_image_formats() {
        assert_eq!(extension(&[0x89, b'P', b'N', b'G', 0]), "png");
        assert_eq!(extension(&[0xFF, 0xD8, 0xFF]), "jpg");
    }
}

// ---------------------------------------------------------------------------------------------
// Several images at once from one prompt, with a shape, quality, optional seed and an optional
// reference image (LobeHub's image studio), for the generate_image tool.

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioRequest {
    pub prompt: String,
    /// An image model (`vendor/model`); the default image model when empty.
    #[serde(default)]
    pub model: Option<String>,
    /// `square`, `landscape` or `portrait`.
    #[serde(default)]
    pub shape: String,
    /// `low`, `medium`, `high` or `auto`.
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default = "one")]
    pub count: u32,
    #[serde(default)]
    pub seed: Option<u64>,
    /// A file in the gallery (`images/…`) to edit or vary.
    #[serde(default)]
    pub reference: Option<String>,
}

fn one() -> u32 {
    1
}

pub(in crate::chat::multimedia) fn studio_size(shape: &str) -> (u32, u32) {
    match shape {
        "landscape" => (1536, 1024),
        "portrait" => (1024, 1536),
        _ => (1024, 1024),
    }
}

/// xAI takes the shape as an aspect ratio, which rig's request leaves at 1:1.
pub(in crate::chat::multimedia) fn aspect_for(
    vendor: &str,
    width: u32,
    height: u32,
) -> Option<&'static str> {
    (vendor == "xai").then(|| match width.cmp(&height) {
        std::cmp::Ordering::Greater => "3:2",
        std::cmp::Ordering::Less => "2:3",
        std::cmp::Ordering::Equal => "1:1",
    })
}

fn merge(a: Option<Value>, b: Option<Value>) -> Option<Value> {
    match (a, b) {
        (Some(Value::Object(mut x)), Some(Value::Object(y))) => {
            x.extend(y);
            Some(Value::Object(x))
        }
        (x, None) => x,
        (None, y) => y,
        (x, _) => x,
    }
}

/// Extra request fields: quality, and a seed for models that take one.
pub(in crate::chat::multimedia) fn studio_params(
    quality: Option<&str>,
    seed: Option<u64>,
) -> Option<Value> {
    let mut p = serde_json::Map::new();
    if let Some(q) = quality.filter(|q| matches!(*q, "low" | "medium" | "high" | "auto")) {
        p.insert("quality".into(), json!(q));
    }
    if let Some(s) = seed {
        p.insert("seed".into(), json!(s));
    }
    (!p.is_empty()).then_some(Value::Object(p))
}

/// Makes the images in a conversation's workspace; returns their files, and the errors of
/// those that failed.
pub(in crate::chat::multimedia) async fn make_images(
    config_dir: &Path,
    workspace: &str,
    request: StudioRequest,
) -> Result<(Vec<String>, Vec<String>), ChatError> {
    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(ChatError::Invalid("Describe the image to create.".into()));
    }
    let reference = match request.model.filter(|m| !m.trim().is_empty()) {
        Some(m) => m,
        None => image_model(&providers::load(config_dir))
            .ok_or_else(|| ChatError::Invalid("No provider that makes images is set up.".into()))?,
    };
    let (width, height) = studio_size(&request.shape);
    let image = match request
        .reference
        .as_deref()
        .filter(|r| !r.trim().is_empty())
    {
        Some(r) => Some(reference_image(config_dir, workspace, r)?),
        None => None,
    };
    let count = request.count.clamp(1, 4);
    let dir = crate::chat::sandbox::workspace(config_dir, workspace)
        .map_err(ChatError::Invalid)?
        .join("images");
    std::fs::create_dir_all(&dir).map_err(ChatError::internal)?;
    if reference.starts_with(SDWEBUI_PREFIX) {
        let server = image_server(config_dir).ok_or_else(|| {
            ChatError::Invalid(
                "Set the local image server's address in Settings → Providers.".into(),
            )
        })?;
        let init = image.as_ref().and_then(image_b64);
        let (w, h) = (width * 2 / 3, height * 2 / 3);
        let mut files = Vec::new();
        let mut errors = Vec::new();
        // One at a time: a local GPU makes images in turn anyway.
        for i in 0..count {
            match sdwebui_image(
                &server,
                &reference,
                &prompt,
                w,
                h,
                request.seed.map(|s| s + u64::from(i)),
                init.as_deref(),
            )
            .await
            {
                Ok(bytes) => {
                    let name = format!(
                        "{}-{}.{}",
                        chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"),
                        i + 1,
                        extension(&bytes)
                    );
                    crate::chat::at_rest::write(dir.join(&name), &bytes)
                        .map_err(ChatError::internal)?;
                    files.push(format!("images/{name}"));
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        if files.is_empty() {
            return Err(ChatError::Invalid(
                errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "No image was made.".into()),
            ));
        }
        return Ok((files, errors));
    }
    let (client, model) = connect(config_dir, &reference).await?;
    let calls = (0..count).map(|i| {
        let mut builder = ImageGenerationRequestBuilder::new(prompt.clone())
            .width(width)
            .height(height);
        // With a seed, each image of the batch gets the next one, so a batch stays reproducible.
        let vendor = providers::split_reference(&reference).map_or("", |(v, _)| v);
        let aspect = aspect_for(vendor, width, height).map(|a| json!({ "aspect_ratio": a }));
        builder = builder.additional_params(merge(
            studio_params(
                request.quality.as_deref(),
                request.seed.map(|s| s + u64::from(i)),
            ),
            aspect,
        ));
        if let Some(img) = &image {
            builder = builder.image(img.clone());
        }
        let request = builder.build();
        let client = client.clone();
        let model = model.clone();
        async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(240),
                client.image_generation(model).call(request),
            )
            .await
        }
    });
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for (i, result) in futures::future::join_all(calls)
        .await
        .into_iter()
        .enumerate()
    {
        match result {
            Ok(Ok(r)) if !r.image.is_empty() => {
                let name = format!("{}-{}.{}", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"), i + 1, extension(&r.image));
                crate::chat::at_rest::write(dir.join(&name), &r.image).map_err(ChatError::internal)?;
                files.push(format!("images/{name}"));
            }
            Ok(Ok(_)) => errors.push("The provider sent back no image.".into()),
            Ok(Err(e)) if e.to_string().contains("takes no reference images") => errors.push(format!("{reference} cannot edit images; start from nothing, or choose a model that edits (OpenAI gpt-image, or a local image server).")),
            Ok(Err(e)) => errors.push(cortex_kernel::context::short_error(&e.to_string())),
            Err(_) => errors.push("An image took too long.".into()),
        }
    }
    if files.is_empty() {
        return Err(ChatError::Invalid(
            errors
                .first()
                .cloned()
                .unwrap_or_else(|| "No image was made.".into()),
        ));
    }
    Ok((files, errors))
}

#[cfg(test)]
mod studio_tests {
    use super::*;

    #[test]
    fn studio_sizes_and_params() {
        assert_eq!(studio_size("landscape"), (1536, 1024));
        assert_eq!(studio_size("portrait"), (1024, 1536));
        assert_eq!(studio_size("anything"), (1024, 1024));
        assert_eq!(studio_params(None, None), None);
        assert_eq!(
            studio_params(Some("high"), Some(7)),
            Some(json!({"quality": "high", "seed": 7}))
        );
        assert_eq!(
            studio_params(Some("ultra"), None),
            None,
            "unknown qualities are not sent"
        );
        assert_eq!(aspect_for("xai", 1024, 1536), Some("2:3"));
        assert_eq!(aspect_for("xai", 1536, 1024), Some("3:2"));
        assert_eq!(
            aspect_for("openai", 1536, 1024),
            None,
            "OpenAI takes the size itself"
        );
        assert_eq!(
            merge(
                Some(json!({"seed": 1})),
                Some(json!({"aspect_ratio": "2:3"}))
            ),
            Some(json!({"seed": 1, "aspect_ratio": "2:3"}))
        );
        let body = sdwebui_body("a fox", 680, 1024, Some(5), Some("dreamshaper_8"), None);
        assert_eq!(
            (body["width"].clone(), body["seed"].clone()),
            (json!(680), json!(5))
        );
        assert_eq!(
            body["override_settings"]["sd_model_checkpoint"],
            "dreamshaper_8"
        );
        assert_eq!(
            sdwebui_body("x", 1, 1, None, Some("default"), Some("AAA"))["init_images"],
            json!(["AAA"])
        );
        assert_eq!(sdwebui_body("x", 1, 1, None, None, None)["seed"], json!(-1));
        let mut file = ProvidersFile::default();
        assert_eq!(image_model(&file), None);
        file.image_server = Some("http://127.0.0.1:7860".into());
        assert_eq!(
            image_model(&file).as_deref(),
            Some("sdwebui/default"),
            "a local server needs no key"
        );
        file.image_model = Some("sdwebui/dreamshaper_8".into());
        assert_eq!(image_model(&file).as_deref(), Some("sdwebui/dreamshaper_8"));
    }
}
