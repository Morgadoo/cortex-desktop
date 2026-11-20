//! The `generate_video` tool: short clips from a description, or an image brought to life, through
//! xAI's video endpoint (Grok Imagine). The job is started, then polled until the clip is ready;
//! the clip is saved in the conversation's workspace (`videos/`), where the chat plays it and
//! run_code can use it.
//!
//! API: `POST {base}/videos/generations` with `model`, `prompt`, `duration`, `aspect_ratio` and
//! an optional `image: {url}` returns `{request_id}`; `GET {base}/videos/{request_id}` returns
//! `{status, video: {url}}`, the status ending as `done`, `failed` or `expired`.

use std::path::Path;
use std::time::Duration;

use rig_core::completion::message::MimeType;
use serde_json::{json, Value};

use crate::chat::providers::{self, ProvidersFile};
use cortex_kernel::tool::{ChatTool, Outcome};

pub const GENERATE_VIDEO: &str = "generate_video";

/// Providers with a video endpoint, with the model used when none is chosen.
const VIDEO_DEFAULTS: &[(&str, &str)] = &[("xai", "grok-imagine-video")];

/// How long a clip may take before the tool gives up.
const WAIT: Duration = Duration::from_secs(600);

pub(in crate::chat::multimedia) fn video_model(file: &ProvidersFile) -> Option<String> {
    if let Some(chosen) = file.video_model.as_ref().filter(|c| !c.trim().is_empty()) {
        let vendor = providers::split_reference(chosen)?.0;
        return (crate::chat::media::usable(file, vendor)
            && VIDEO_DEFAULTS.iter().any(|(v, _)| *v == vendor))
        .then(|| chosen.clone());
    }
    VIDEO_DEFAULTS
        .iter()
        .find(|(vendor, _)| crate::chat::media::usable(file, vendor))
        .map(|(vendor, model)| format!("{vendor}/{model}"))
}

pub(in crate::chat::multimedia) fn vendors(file: &ProvidersFile) -> Vec<String> {
    VIDEO_DEFAULTS
        .iter()
        .filter(|(v, _)| crate::chat::media::usable(file, v))
        .map(|(v, _)| v.to_string())
        .collect()
}

/// The tool, when a provider can make videos.
pub fn definition(config_dir: &Path) -> Option<ChatTool> {
    let model = video_model(&providers::load(config_dir))?;
    Some(ChatTool {
        name: GENERATE_VIDEO.into(),
        description: format!(
            "Create a short video clip (1–15 seconds) from a description, or animate an image. \
             Describe the subject, the motion, the camera and the mood. To bring an image to \
             life, pass it as `reference` (`attached`, or a `file` from generate_image). The \
             clip is shown to the user and saved in this conversation's workspace. Takes a minute \
             or more. Uses {model}."
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "What happens in the clip: subject, motion, camera, style." },
                "seconds": { "type": "integer", "minimum": 1, "maximum": 15, "description": "Length. Default 6." },
                "shape": { "type": "string", "enum": ["landscape", "portrait", "square"], "default": "landscape" },
                "reference": { "type": "string", "description": "An image to start from: `attached`, or images/... from generate_image." }
            },
            "required": ["prompt"]
        }),
        connector_id: None,
        tool_name: GENERATE_VIDEO.into(),
        read_only: true,
    })
}

pub(in crate::chat::multimedia) fn aspect(shape: &str) -> &'static str {
    match shape {
        "portrait" => "9:16",
        "square" => "1:1",
        _ => "16:9",
    }
}

/// The request body that starts a clip.
pub(in crate::chat::multimedia) fn start_body(
    model: &str,
    prompt: &str,
    seconds: u64,
    shape: &str,
    image_url: Option<&str>,
) -> Value {
    let mut body = json!({ "model": model, "prompt": prompt, "duration": seconds.clamp(1, 15), "aspect_ratio": aspect(shape) });
    if let Some(url) = image_url {
        body["image"] = json!({ "url": url });
    }
    body
}

/// What a poll says: the clip's address when done, an error when it ended without one, or
/// `None` while it is still being made.
pub(in crate::chat::multimedia) fn poll_state(body: &Value) -> Option<Result<String, String>> {
    let status = body.get("status").and_then(Value::as_str).unwrap_or("");
    let url = body
        .pointer("/video/url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty());
    match (status, url) {
        (_, Some(url)) if status == "done" || status.is_empty() => Some(Ok(url.to_string())),
        ("done", None) => Some(Err("The provider finished but sent no video.".into())),
        ("failed", _) => Some(Err(body
            .pointer("/error/message")
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .map_or("The video could not be made.".into(), |e| {
                format!("The video could not be made: {e}")
            }))),
        ("expired", _) => Some(Err("The video request expired before it was made.".into())),
        _ => None,
    }
}

/// The service's address with its `/v1`, which the video paths sit under.
fn base_url(file: &ProvidersFile, vendor: &str) -> String {
    let url = file
        .providers
        .get(vendor)
        .and_then(|s| s.base_url.clone())
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "https://api.x.ai/v1".into());
    let url = url.trim().trim_end_matches('/');
    if url.ends_with("/v1") {
        url.to_string()
    } else {
        format!("{url}/v1")
    }
}

async fn key(file: &ProvidersFile, vendor: &str) -> Result<String, String> {
    let saved = if file.providers.get(vendor).is_some_and(|s| s.has_key) {
        crate::chat::providers::api_key(vendor).await?
    } else {
        None
    };
    saved
        .filter(|k| !k.is_empty())
        .or_else(|| providers::env_key(vendor).and_then(|name| std::env::var(name).ok()))
        .ok_or_else(|| format!("Add an API key for {vendor} in Settings → Providers."))
}

/// Starts a clip at `base`, waits for it and downloads it.
pub(in crate::chat::multimedia) async fn make_at(
    base: &str,
    key: &str,
    body: &Value,
    poll_every: Duration,
) -> Result<Vec<u8>, String> {
    let client = crate::support::http::client(Duration::from_secs(120))?;
    let started = client
        .post(format!("{base}/videos/generations"))
        .bearer_auth(key)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("The video service did not answer: {e}"))?;
    let status = started.status();
    let answer: Value = started.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let why = answer.get("error").and_then(|e| {
            e.as_str()
                .map(str::to_string)
                .or_else(|| e.get("message").and_then(Value::as_str).map(str::to_string))
        });
        return Err(cortex_kernel::context::short_error(&format!(
            "The video service answered {status}{}",
            why.map(|w| format!(": {w}")).unwrap_or_default()
        )));
    }
    // Some services answer at once with the clip.
    let url = match poll_state(&answer) {
        Some(done) => done?,
        None => {
            let id = answer
                .get("request_id")
                .or_else(|| answer.get("id"))
                .and_then(Value::as_str)
                .ok_or("The video service sent no request id.")?
                .to_string();
            let deadline = tokio::time::Instant::now() + WAIT;
            loop {
                tokio::time::sleep(poll_every).await;
                if tokio::time::Instant::now() > deadline {
                    return Err("The video took too long; try a shorter clip.".into());
                }
                let r = client
                    .get(format!("{base}/videos/{id}"))
                    .bearer_auth(key)
                    .send()
                    .await;
                let Ok(r) = r else { continue };
                let body: Value = r.json().await.unwrap_or(Value::Null);
                if let Some(done) = poll_state(&body) {
                    break done?;
                }
            }
        }
    };
    let bytes = if let Some(b64) = url
        .strip_prefix("data:")
        .and_then(|d| d.split_once(";base64,"))
        .map(|(_, b)| b)
    {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| e.to_string())?
    } else {
        client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("The video could not be downloaded: {e}"))?
            .bytes()
            .await
            .map_err(|e| e.to_string())?
            .to_vec()
    };
    if bytes.len() < 64 {
        return Err("The video file was empty.".into());
    }
    Ok(bytes)
}

fn extension(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x1A, 0x45, 0xDF, 0xA3, ..] => "webm",
        [_, _, _, _, b'f', b't', b'y', b'p', b'q', b't', ..] => "mov",
        _ => "mp4",
    }
}

/// The tool: makes the clip and saves it in the conversation's workspace.
pub async fn generate(config_dir: &Path, conversation_id: &str, args: &Value) -> Outcome {
    match run(config_dir, conversation_id, args).await {
        Ok(text) => Outcome {
            text,
            is_error: false,
        },
        Err(e) => Outcome::err(e),
    }
}

async fn run(config_dir: &Path, conversation_id: &str, args: &Value) -> Result<String, String> {
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let prompt = s("prompt").ok_or("Describe the video to create.")?;
    let seconds = args.get("seconds").and_then(Value::as_u64).unwrap_or(6);
    let shape = s("shape").unwrap_or_else(|| "landscape".into());
    let file = providers::load(config_dir);
    let reference =
        video_model(&file).ok_or("No provider that makes videos is set up (xAI makes them).")?;
    let (vendor, model) =
        providers::split_reference(&reference).ok_or("The video model is not vendor/model.")?;
    let image_url = match s("reference") {
        Some(r) => {
            let image = crate::chat::media::reference_image(config_dir, conversation_id, &r)?;
            let data = crate::chat::media::image_b64(&image)
                .ok_or("The reference image could not be read.")?;
            let mime = image
                .media_type
                .as_ref()
                .map_or("image/png", |m| m.to_mime_type());
            Some(format!("data:{mime};base64,{data}"))
        }
        None => None,
    };
    let body = start_body(model, &prompt, seconds, &shape, image_url.as_deref());
    let bytes = make_at(
        &base_url(&file, vendor),
        &key(&file, vendor).await?,
        &body,
        Duration::from_secs(5),
    )
    .await?;
    let dir = crate::chat::sandbox::workspace(config_dir, conversation_id)?.join("videos");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!(
        "{}.{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        extension(&bytes)
    );
    crate::chat::at_rest::write(dir.join(&name), &bytes).map_err(|e| e.to_string())?;
    Ok(json!({ "file": format!("videos/{name}"), "model": reference, "seconds": seconds.clamp(1, 15), "shape": shape }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_and_poll_states() {
        let b = start_body(
            "grok-imagine-video",
            "a boat",
            40,
            "portrait",
            Some("data:image/png;base64,AA"),
        );
        assert_eq!(b["duration"], 15);
        assert_eq!(b["aspect_ratio"], "9:16");
        assert_eq!(b["image"]["url"], "data:image/png;base64,AA");
        assert!(start_body("m", "p", 6, "x", None).get("image").is_none());
        assert_eq!(poll_state(&json!({"status": "pending"})), None);
        assert_eq!(
            poll_state(&json!({"status": "done", "video": {"url": "https://v/1.mp4"}})),
            Some(Ok("https://v/1.mp4".into()))
        );
        assert!(poll_state(&json!({"status": "expired"})).unwrap().is_err());
        assert!(poll_state(&json!({"status": "failed", "error": "nsfw"}))
            .unwrap()
            .unwrap_err()
            .contains("nsfw"));
    }

    #[tokio::test]
    async fn starts_polls_and_downloads() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let video_url = format!("{base}/files/clip.mp4");
        let server = tokio::spawn(async move {
            let mut seen = Vec::new();
            for answer in [
                r#"{"request_id":"r1"}"#.to_string(),
                r#"{"status":"pending"}"#.to_string(),
                format!(r#"{{"status":"done","video":{{"url":"{video_url}"}}}}"#),
                "BINARY".into(),
            ] {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap();
                seen.push(String::from_utf8_lossy(&buf[..n]).to_string());
                let body: Vec<u8> = if answer == "BINARY" {
                    [b"\0\0\0\x18ftypmp42".as_slice(), &[7u8; 100]].concat()
                } else {
                    answer.into_bytes()
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                s.write_all(head.as_bytes()).await.unwrap();
                s.write_all(&body).await.unwrap();
            }
            seen
        });
        let body = start_body("grok-imagine-video", "a paper boat", 6, "landscape", None);
        let bytes = make_at(&base, "k", &body, Duration::from_millis(10))
            .await
            .unwrap();
        assert_eq!(extension(&bytes), "mp4");
        let seen = server.await.unwrap();
        assert!(
            seen[0].starts_with("POST /videos/generations")
                && seen[0].contains("a paper boat")
                && seen[0].to_lowercase().contains("authorization: bearer k")
        );
        assert!(seen[1].starts_with("GET /videos/r1") && seen[2].starts_with("GET /videos/r1"));
        assert!(seen[3].starts_with("GET /files/clip.mp4"));
    }
}
