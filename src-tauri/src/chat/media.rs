//! Images and speech: the `generate_image` tool and voice input, through rig's OpenAI-shaped
//! image and transcription models.
//!
//! Each uses a provider the user already set up, with its key and base URL. The model is the
//! one chosen in settings, or a sensible default for the first provider that offers one.

use std::path::Path;

use rig_core::audio_generation::AudioGenerationRequestBuilder;
use rig_core::image_generation::ImageGenerationRequestBuilder;
use rig_core::providers::openai::wire::{self as dialects, Dialect};
use rig_core::providers::openai::{OpenAI, OpenAIConfig};
use rig_core::providers::xai;
use rig_core::transcription::TranscriptionRequestBuilder;
use serde::Serialize;
use serde_json::{json, Value};

use super::providers::{self, ProviderSettings, ProvidersFile};
use super::tools::{ChatTool, Outcome};

pub const GENERATE_IMAGE: &str = "generate_image";

/// Providers whose image endpoint rig speaks, with the model used when none is chosen.
const IMAGE_DEFAULTS: &[(&str, &str)] = &[("openai", "gpt-image-1"), ("xai", "grok-imagine-image"), ("together", "black-forest-labs/FLUX.1-schnell")];
/// Providers that read text aloud, with the model used when none is chosen.
const VOICE_DEFAULTS: &[(&str, &str)] = &[("openai", "gpt-4o-mini-tts"), ("xai", "grok-tts")];

/// Providers with a speech-to-text endpoint, with the model used when none is chosen.
const SPEECH_DEFAULTS: &[(&str, &str)] = &[("openai", "gpt-4o-mini-transcribe"), ("groq", "whisper-large-v3-turbo")];

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
fn usable(file: &ProvidersFile, vendor: &str) -> bool {
    file.providers
        .get(vendor)
        .is_some_and(|s| s.enabled && (s.has_key || providers::env_key(vendor).is_some()))
}

fn pick(file: &ProvidersFile, chosen: Option<&String>, defaults: &[(&str, &str)]) -> Option<String> {
    if let Some(chosen) = chosen.filter(|c| !c.trim().is_empty()) {
        let vendor = providers::split_reference(chosen)?.0;
        return (usable(file, vendor) && dialect(vendor).is_some()).then(|| chosen.clone());
    }
    defaults
        .iter()
        .find(|(vendor, _)| usable(file, vendor))
        .map(|(vendor, model)| format!("{vendor}/{model}"))
}

pub fn image_model(file: &ProvidersFile) -> Option<String> {
    pick(file, file.image_model.as_ref(), IMAGE_DEFAULTS)
}

pub fn speech_model(file: &ProvidersFile) -> Option<String> {
    pick(file, file.speech_model.as_ref(), SPEECH_DEFAULTS)
}

pub fn voice_model(file: &ProvidersFile) -> Option<String> {
    pick(file, file.voice_model.as_ref(), VOICE_DEFAULTS)
}

pub fn models(config_dir: &Path) -> MediaModels {
    let file = providers::load(config_dir);
    let vendors = |defaults: &[(&str, &str)]| defaults.iter().filter(|(v, _)| usable(&file, v)).map(|(v, _)| v.to_string()).collect();
    MediaModels {
        image_model: image_model(&file),
        speech_model: speech_model(&file),
        voice_model: voice_model(&file),
        image_vendors: vendors(IMAGE_DEFAULTS),
        speech_vendors: vendors(SPEECH_DEFAULTS),
        voice_vendors: vendors(VOICE_DEFAULTS),
    }
}

/// An OpenAI-shaped client for `vendor`, with the user's key and base URL.
fn client(vendor: &str, settings: &ProviderSettings, key: Option<String>) -> Result<OpenAI, String> {
    let dialect = dialect(vendor).ok_or_else(|| format!("{vendor} has no image or speech endpoint here."))?;
    let key = key
        .filter(|k| !k.is_empty())
        .or_else(|| providers::env_key(vendor).and_then(|name| std::env::var(name).ok()))
        .ok_or_else(|| format!("Add an API key for {vendor} in Settings → Providers."))?;
    let mut config = OpenAIConfig::with_key(dialect, key);
    if let Some(base_url) = settings.base_url.as_ref().filter(|u| !u.trim().is_empty()) {
        config = config.with_base_url(providers::normalize_base_url(vendor, base_url));
    }
    Ok(config.client())
}

async fn connect(config_dir: &Path, reference: &str) -> Result<(OpenAI, String), String> {
    let (vendor, model) = providers::split_reference(reference).ok_or_else(|| format!("`{reference}` is not vendor/model"))?;
    let settings = providers::load(config_dir).providers.get(vendor).cloned().unwrap_or_default();
    let key = if settings.has_key { super::engine::api_key(vendor).await? } else { None };
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
                "reference": { "type": "string", "description": "To edit or vary an image: `attached` for the latest image the user attached, or the `file` an earlier generate_image call returned (images/...)." }
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

/// Generates an image and saves it in the conversation's workspace.
pub async fn generate(config_dir: &Path, conversation_id: &str, args: &Value) -> Outcome {
    let err = |text: String| Outcome { text, is_error: true };
    let Some(prompt) = args.get("prompt").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) else {
        return err("Describe the image to create.".into());
    };
    let Some(reference) = image_model(&providers::load(config_dir)) else {
        return err("No provider that makes images is set up.".into());
    };
    let (client, model) = match connect(config_dir, &reference).await {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let (width, height) = match args.get("shape").and_then(Value::as_str) {
        Some("landscape") => (1536, 1024),
        Some("portrait") => (1024, 1536),
        _ => (1024, 1024),
    };
    let mut builder = ImageGenerationRequestBuilder::new(prompt).width(width).height(height);
    if let Some(reference) = args.get("reference").and_then(Value::as_str).filter(|r| !r.trim().is_empty()) {
        match reference_image(config_dir, conversation_id, reference.trim()) {
            Ok(image) => builder = builder.image(image),
            Err(e) => return err(e),
        }
    }
    let request = builder.build();
    let image = match tokio::time::timeout(std::time::Duration::from_secs(180), client.image_generation(model.clone()).call(request)).await {
        Ok(Ok(response)) => response.image,
        Ok(Err(e)) if e.to_string().contains("takes no reference images") => {
            return err(format!("{reference} cannot edit images. Make a new image from a fuller description instead, or set an image model that edits (OpenAI gpt-image-1) in Settings → Providers."))
        }
        Ok(Err(e)) => return err(format!("The image could not be made: {e}")),
        Err(_) => return err("The image took too long to make.".into()),
    };
    if image.is_empty() {
        return err("The provider sent back no image.".into());
    }
    let dir = match super::sandbox::workspace(config_dir, conversation_id) {
        Ok(d) => d.join("images"),
        Err(e) => return err(e),
    };
    let name = format!("{}.{}", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"), extension(&image));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| crate::chat::at_rest::write(dir.join(&name), &image)) {
        return err(format!("Could not save the image: {e}"));
    }
    Outcome {
        text: json!({ "file": format!("images/{name}"), "model": reference, "bytes": image.len() }).to_string(),
        is_error: false,
    }
}

/// The image an edit starts from: the user's latest attachment, or an image in the workspace.
fn reference_image(config_dir: &Path, conversation_id: &str, reference: &str) -> Result<rig_core::completion::message::Image, String> {
    use base64::Engine as _;
    use rig_core::completion::message::{DocumentSourceKind, Image, ImageMediaType, Message, UserContent};
    let (data, media_type) = if reference == "attached" {
        let conversation = super::store::load_conversation(config_dir, conversation_id)?;
        conversation
            .messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { content } => content.iter().find_map(|c| match c {
                    UserContent::Image(image) => match &image.data {
                        DocumentSourceKind::Base64(b64) => Some((b64.clone(), image.media_type.clone())),
                        _ => None,
                    },
                    _ => None,
                }),
                _ => None,
            })
            .ok_or("No image was attached in this conversation.")?
    } else {
        // Only files inside this conversation's workspace.
        let root = super::sandbox::workspace(config_dir, conversation_id)?;
        let canonical = root.join(reference).canonicalize().map_err(|_| format!("There is no image at {reference}."))?;
        if !canonical.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
            return Err("The reference must be an image in this conversation's workspace.".into());
        }
        let bytes = crate::chat::at_rest::read(&canonical).map_err(|e| e.to_string())?;
        let media = ImageMediaType::sniff(&bytes);
        (base64::engine::general_purpose::STANDARD.encode(bytes), media)
    };
    Ok(Image { data: DocumentSourceKind::Base64(data), media_type, detail: None, native: None })
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
        if in_code || trimmed.starts_with('|') && trimmed.matches('|').count() > 2 && trimmed.contains("---") {
            continue;
        }
        let line = trimmed.trim_start_matches(['#', '>', '-', '*', '+']).trim_start();
        out.push_str(line);
        out.push('\n');
    }
    // [text](url) → text; drop emphasis marks and inline code ticks.
    let links = regex::Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("valid regex");
    let mut text = links.replace_all(&out, "$1").replace(['*', '_', '`'], "");
    if text.chars().count() > SPEAK_CHARS {
        let cut: String = text.chars().take(SPEAK_CHARS).collect();
        let end = cut.rfind(['.', '!', '?']).map_or(cut.len(), |i| i + 1);
        text = cut[..end].to_string();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads `text` aloud with the chosen voice model; returns the audio and its media type.
pub async fn speak(config_dir: &Path, text: &str) -> Result<(Vec<u8>, String), String> {
    let reference = voice_model(&providers::load(config_dir))
        .ok_or("Read aloud needs a provider with text-to-speech (OpenAI or xAI) in Settings → Providers.")?;
    let (vendor, _) = providers::split_reference(&reference).unwrap_or_default();
    let (client, model) = connect(config_dir, &reference).await?;
    let text = speakable(text);
    if text.is_empty() {
        return Err("There is nothing to read.".into());
    }
    let voice = if vendor == "xai" { "eve" } else { "alloy" };
    let request = AudioGenerationRequestBuilder::new(text, voice).build();
    let response = tokio::time::timeout(std::time::Duration::from_secs(120), client.audio_generation(model).call(request))
        .await
        .map_err(|_| "Reading aloud took too long.".to_string())?
        .map_err(|e| e.to_string())?;
    let mime = match response.audio.as_slice() {
        [b'R', b'I', b'F', b'F', ..] => "audio/wav",
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        _ => "audio/mpeg",
    };
    Ok((response.audio, mime.into()))
}

/// Turns recorded speech into text with the chosen speech model.
pub async fn transcribe(config_dir: &Path, audio: Vec<u8>, file_name: &str) -> Result<String, String> {
    let reference = speech_model(&providers::load(config_dir))
        .ok_or("Voice input needs a provider with speech-to-text (OpenAI or Groq) in Settings → Providers.")?;
    let (client, model) = connect(config_dir, &reference).await?;
    let request = TranscriptionRequestBuilder::new(audio).filename(Some(file_name.to_string())).build();
    let response = client.transcription(model).call(request).await.map_err(|e| e.to_string())?;
    Ok(response.text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(vendors: &[&str]) -> ProvidersFile {
        let mut f = ProvidersFile::default();
        for v in vendors {
            f.providers.insert(v.to_string(), ProviderSettings { enabled: true, has_key: true, ..Default::default() });
        }
        f
    }

    #[test]
    fn picks_a_model_from_the_providers_set_up() {
        assert_eq!(image_model(&file(&["xai"])).as_deref(), Some("xai/grok-imagine-image"));
        assert_eq!(image_model(&file(&["openai", "xai"])).as_deref(), Some("openai/gpt-image-1"));
        assert_eq!(image_model(&file(&["anthropic"])), None);
        assert_eq!(speech_model(&file(&["groq"])).as_deref(), Some("groq/whisper-large-v3-turbo"));
        let mut chosen = file(&["openai", "xai"]);
        chosen.image_model = Some("xai/grok-imagine-image-pro".into());
        assert_eq!(image_model(&chosen).as_deref(), Some("xai/grok-imagine-image-pro"));
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
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
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
        let settings = ProviderSettings { enabled: true, has_key: true, base_url: Some(url), custom_models: vec![] };
        let client = client("openai", &settings, Some("test-key".into())).unwrap();
        let request = TranscriptionRequestBuilder::new(b"RIFFfake".to_vec()).filename(Some("speech.webm".into())).build();
        let response = client.transcription("whisper-1").call(request).await.unwrap();
        assert_eq!(response.text.trim(), "hello there");
        let asked = server.join().unwrap();
        assert!(asked.starts_with("POST /v1/audio/transcriptions"), "{asked}");
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
