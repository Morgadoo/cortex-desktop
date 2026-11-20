//! Hex (github.com/anomalyco/hex), the local dictation app, as a speech-to-text engine.
//!
//! Hex runs its speech models (Parakeet, Whisper and others) on this computer and serves them
//! on its local API, version 2: an owner-only discovery file, `local-api.json` in Hex's data
//! folder, holds the port and a bearer token; `GET /health` and `GET /models` say whether it is
//! ready and with what; `POST /transcriptions?model=<id>&language=<code>` takes 16 kHz mono WAV
//! of at most 30 seconds and answers `{"transcript": …}`. Longer recordings go in pieces.
//!
//! "Hex isn't running" covers a missing discovery file, a refused connection, a rejected token
//! and any API version but 2; then the built-in Whisper takes over (`audio::transcribe`).
//! Nothing here logs or shows the token.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Tried in this order when the user picked no model, or Hex lacks the one they picked.
pub const PREFERRED_MODELS: [&str; 4] = [
    "parakeet_unified_en",
    "parakeet_v3",
    "parakeet_v2",
    "whisper_large_v3_turbo",
];

const SAMPLE_RATE: usize = 16_000;
/// The longest piece Hex takes in one request.
const PIECE_SECONDS: usize = 30;
/// Busy (`429 queue-full`) answers retried before giving up, and the wait between them.
const BUSY_RETRIES: u32 = 5;
const BUSY_BACKOFF: Duration = Duration::from_millis(200);

/// Where Hex is listening, from its discovery file.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    token: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Endpoint {{ port: {}, token: <hidden> }}", self.port)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Discovery {
    port: u16,
    token: String,
    api_version: String,
}

/// Hex's data folder: `HEX_APPLICATION_SUPPORT_DIR` when set, else the system's per-user data
/// folder (`~/.local/share/hex` or `$XDG_DATA_HOME/hex` on Linux, `~/Library/Application
/// Support/hex` on macOS, `%APPDATA%\hex` on Windows). `env` reads a variable.
pub fn support_dir(os: &str, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(dir) = env("HEX_APPLICATION_SUPPORT_DIR").filter(|d| !d.is_empty()) {
        return Some(dir.into());
    }
    let home = || env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let data = match os {
        "macos" => home()?.join("Library").join("Application Support"),
        "windows" => PathBuf::from(env("APPDATA").filter(|a| !a.is_empty())?),
        _ => env("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local").join("share")))?,
    };
    Some(data.join("hex"))
}

/// This computer's discovery file.
pub fn discovery_path() -> Option<PathBuf> {
    support_dir(std::env::consts::OS, |k| std::env::var(k).ok()).map(|d| d.join("local-api.json"))
}

/// Reads a discovery file; the reason it can't be used is never the token itself.
pub fn parse_discovery(bytes: &[u8]) -> Result<Endpoint, String> {
    let Ok(d) = serde_json::from_slice::<Discovery>(bytes) else {
        return Err("Hex's discovery file could not be read.".into());
    };
    if d.api_version != "2" {
        return Err(format!(
            "Hex's local API is version {}; Cortex speaks version 2. Update Hex or Cortex.",
            d.api_version
        ));
    }
    if d.port == 0 || d.token.is_empty() || d.token.contains(['\r', '\n']) {
        return Err("Hex's discovery file has no usable port or token.".into());
    }
    Ok(Endpoint {
        port: d.port,
        token: d.token,
    })
}

/// A model Hex has, as `GET /models` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub verified: bool,
}

impl Model {
    /// Downloaded and checked: Hex can transcribe with it now. Cortex never asks Hex to
    /// download or prepare one.
    pub fn usable(&self) -> bool {
        self.installed && self.verified
    }
}

/// The user's model when Hex has it ready, else the first ready one of [`PREFERRED_MODELS`],
/// else any ready one.
pub fn pick_model(models: &[Model], chosen: &str) -> Option<String> {
    let ready = |id: &str| models.iter().any(|m| m.id == id && m.usable());
    Some(chosen)
        .filter(|id| !id.is_empty() && ready(id))
        .or_else(|| PREFERRED_MODELS.into_iter().find(|id| ready(id)))
        .map(str::to_string)
        .or_else(|| models.iter().find(|m| m.usable()).map(|m| m.id.clone()))
}

/// Why Hex could not transcribe.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// Not running, not reachable, or not speaking API 2: the built-in Whisper can take over.
    Unavailable(String),
    /// Hex answered and refused (a bad model or audio, a full queue that stayed full).
    Refused(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unavailable(m) | Failure::Refused(m) => f.write_str(m),
        }
    }
}

/// The usual reason Hex can't be reached: it isn't open. Settings words this itself, in the
/// interface's language, so `Status::problem` leaves it out.
const NOT_RUNNING: &str = "Hex isn't running.";

/// What Settings shows about Hex.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub version: Option<String>,
    pub models: Vec<Model>,
    /// Why it can't be used, when that is more than Hex simply not being open.
    pub problem: Option<String>,
}

pub struct Client {
    endpoint: Endpoint,
    http: reqwest::Client,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    version: String,
    api_version: String,
}

#[derive(Deserialize)]
struct Transcription {
    transcript: String,
}

/// The `code` of an error body (`{"code": …}` or `{"error": {"code": …}}`).
fn error_code(body: &[u8]) -> String {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    v.get("code")
        .or_else(|| v.get("error").and_then(|e| e.get("code")))
        .and_then(|c| c.as_str())
        .unwrap_or("unknown")
        .to_string()
}

/// A refusal in words the user can act on.
fn refusal(code: &str) -> String {
    match code {
        "model-not-ready" => {
            "Hex hasn't finished loading that model; try again in a moment, or open Hex to prepare it.".into()
        }
        "unsupported-audio" => "Hex couldn't read this recording.".into(),
        "unsupported-language" => {
            "Hex doesn't know that language; clear the language in Settings → Audio or choose another.".into()
        }
        "queue-full" => "Hex is busy with other transcriptions; try again shortly.".into(),
        "resource-exhausted" => "Hex ran out of memory for this recording; try a smaller model in Hex.".into(),
        other => format!("Hex refused the recording ({other})."),
    }
}

impl Client {
    /// Hex at the discovery file `path`, after checking it answers and speaks API 2.
    pub async fn connect_at(path: &Path) -> Result<Self, Failure> {
        let bytes = std::fs::read(path).map_err(|_| Failure::Unavailable(NOT_RUNNING.into()))?;
        let endpoint = parse_discovery(&bytes).map_err(Failure::Unavailable)?;
        let client = Self {
            endpoint,
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_millis(500))
                .no_proxy()
                .build()
                .map_err(|e| Failure::Unavailable(e.to_string()))?,
        };
        client.health().await?;
        Ok(client)
    }

    pub async fn connect() -> Result<Self, Failure> {
        let path = discovery_path()
            .ok_or_else(|| Failure::Unavailable("Hex's data folder is unknown here.".into()))?;
        Self::connect_at(&path).await
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.endpoint.port)
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>, Failure> {
        let response = self
            .http
            .get(self.url(path))
            .bearer_auth(&self.endpoint.token)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map_err(|_| Failure::Unavailable(NOT_RUNNING.into()))?;
        match response.status().as_u16() {
            200 => Ok(response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .unwrap_or_default()),
            401 => Err(Failure::Unavailable(
                "Hex refused Cortex's credentials; restart Hex.".into(),
            )),
            status => Err(Failure::Unavailable(format!(
                "Hex answered {path} with HTTP {status}."
            ))),
        }
    }

    /// Hex's version, after checking it speaks API 2.
    pub async fn health(&self) -> Result<String, Failure> {
        let health: Health = serde_json::from_slice(&self.get("/health").await?)
            .map_err(|_| Failure::Unavailable("Hex sent an unreadable health answer.".into()))?;
        if health.api_version != "2" {
            return Err(Failure::Unavailable(format!(
                "Hex's local API is version {}; Cortex speaks version 2.",
                health.api_version
            )));
        }
        Ok(health.version)
    }

    pub async fn models(&self) -> Result<Vec<Model>, Failure> {
        serde_json::from_slice(&self.get("/models").await?)
            .map_err(|_| Failure::Unavailable("Hex sent an unreadable model list.".into()))
    }

    /// Transcribes 16 kHz mono `samples` with `model`, in pieces of at most 30 seconds, and
    /// joins the pieces' text.
    pub async fn transcribe(
        &self,
        samples: &[f32],
        model: &str,
        language: &str,
    ) -> Result<String, Failure> {
        let mut texts = Vec::new();
        for piece in samples.chunks(SAMPLE_RATE * PIECE_SECONDS) {
            let text = self.transcribe_piece(&wav(piece), model, language).await?;
            let text = text.trim();
            if !text.is_empty() {
                texts.push(text.to_string());
            }
        }
        Ok(texts.join(" "))
    }

    async fn transcribe_piece(
        &self,
        wav: &[u8],
        model: &str,
        language: &str,
    ) -> Result<String, Failure> {
        let mut attempts = 0;
        loop {
            // Without a language, Hex uses its own (the one its dictation is set to).
            let mut query = vec![("model", model)];
            if !language.is_empty() {
                query.push(("language", language));
            }
            let response = self
                .http
                .post(self.url("/transcriptions"))
                .query(&query)
                .bearer_auth(&self.endpoint.token)
                .header("Content-Type", "audio/wav")
                // A cold model may need to load before the first answer.
                .timeout(Duration::from_secs(90))
                .body(wav.to_vec())
                .send()
                .await
                .map_err(|_| Failure::Unavailable("Hex stopped answering.".into()))?;
            let status = response.status().as_u16();
            let body = response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .unwrap_or_default();
            match status {
                200 => {
                    return serde_json::from_slice::<Transcription>(&body)
                        .map(|t| t.transcript)
                        .map_err(|_| Failure::Refused("Hex sent an unreadable transcript.".into()))
                }
                401 => {
                    return Err(Failure::Unavailable(
                        "Hex refused Cortex's credentials; restart Hex.".into(),
                    ))
                }
                429 if attempts < BUSY_RETRIES => {
                    attempts += 1;
                    tokio::time::sleep(BUSY_BACKOFF).await;
                }
                _ => return Err(Failure::Refused(refusal(&error_code(&body)))),
            }
        }
    }
}

/// What Settings shows: whether Hex is running, its version and its models.
pub async fn status() -> Status {
    status_of(Client::connect().await).await
}

async fn status_of(connected: Result<Client, Failure>) -> Status {
    match connected {
        Ok(client) => {
            let version = client.health().await.ok();
            match client.models().await {
                Ok(models) => Status {
                    running: true,
                    version,
                    models,
                    problem: None,
                },
                Err(e) => Status {
                    running: false,
                    version,
                    models: Vec::new(),
                    problem: Some(e.to_string()),
                },
            }
        }
        Err(e) => Status {
            running: false,
            version: None,
            models: Vec::new(),
            problem: Some(e.to_string()).filter(|p| p != NOT_RUNNING),
        },
    }
}

/// 16-bit PCM WAV, 16 kHz mono, of `samples` in [-1, 1].
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn hex_s_data_folder_is_found_on_each_system() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            support_dir("linux", env(&[("HOME", "/home/a")])),
            Some(PathBuf::from("/home/a/.local/share/hex"))
        );
        assert_eq!(
            support_dir(
                "linux",
                env(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/data")])
            ),
            Some(PathBuf::from("/data/hex"))
        );
        assert_eq!(
            support_dir("macos", env(&[("HOME", "/Users/a")])),
            Some(PathBuf::from("/Users/a/Library/Application Support/hex"))
        );
        assert_eq!(
            support_dir("windows", env(&[("APPDATA", "C:/Users/a/AppData/Roaming")])),
            Some(PathBuf::from("C:/Users/a/AppData/Roaming/hex"))
        );
        assert_eq!(
            support_dir(
                "linux",
                env(&[("HEX_APPLICATION_SUPPORT_DIR", "/x"), ("HOME", "/h")])
            ),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(support_dir("linux", env(&[])), None);
    }

    #[test]
    fn the_discovery_file_must_be_version_2_with_a_port_and_token() {
        let ok =
            parse_discovery(br#"{"port":4123,"token":"t0k","apiVersion":"2","pid":7}"#).unwrap();
        assert_eq!(ok.port, 4123);
        assert!(
            !format!("{ok:?}").contains("t0k"),
            "the token is never shown"
        );
        assert!(
            parse_discovery(br#"{"port":1,"token":"t","apiVersion":"3"}"#)
                .unwrap_err()
                .contains("version 3")
        );
        assert!(parse_discovery(br#"{"port":0,"token":"t","apiVersion":"2"}"#).is_err());
        assert!(parse_discovery(br#"{"port":1,"token":"a\nb","apiVersion":"2"}"#).is_err());
        let err = parse_discovery(br#"{"port":"x","token":"secret-token"}"#).unwrap_err();
        assert!(!err.contains("secret-token"));
    }

    #[test]
    fn the_chosen_model_wins_when_ready_then_the_preferred_ones() {
        let m = |id: &str, ready: bool| Model {
            id: id.into(),
            name: id.into(),
            installed: ready,
            verified: ready,
        };
        let models = [
            m("whisper_large_v3_turbo", true),
            m("parakeet_v3", true),
            m("parakeet_unified_en", false),
            m("sense_voice", true),
        ];
        assert_eq!(
            pick_model(&models, "sense_voice").as_deref(),
            Some("sense_voice")
        );
        assert_eq!(pick_model(&models, "").as_deref(), Some("parakeet_v3"));
        assert_eq!(
            pick_model(&models, "parakeet_unified_en").as_deref(),
            Some("parakeet_v3"),
            "not ready yet"
        );
        assert_eq!(
            pick_model(&[m("other", true)], "").as_deref(),
            Some("other")
        );
        assert_eq!(pick_model(&[m("x", false)], ""), None);
    }

    #[test]
    fn wav_is_16_bit_mono_16k() {
        let w = wav(&[0.0, 1.0, -1.0, 2.0]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 8);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([w[50], w[51]]), i16::MAX, "clamped");
        assert_eq!(
            crate::chat::local_models::audio_samples(&w).unwrap().len(),
            4,
            "the built-in decoder reads it back"
        );
    }

    /// What the fake Hex saw: (method, path with query, authorization, body length).
    type Seen = Arc<Mutex<Vec<(String, String, String, usize)>>>;

    /// A fake Hex on a free port: `/health`, `/models` and `/transcriptions`, which answers
    /// `responses` in order (status, body), then 200 with the piece's length in seconds.
    async fn fake_hex(responses: Vec<(u16, &'static str)>) -> (u16, Seen) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::default();
        let queue = Arc::new(Mutex::new(responses));
        let record = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (record, queue) = (record.clone(), queue.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 65536];
                    // Read the head, then the body by its length.
                    let head_end = loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let header = |name: &str| {
                        head.lines()
                            .filter_map(|l| l.split_once(':'))
                            .find(|(k, _)| k.eq_ignore_ascii_case(name))
                            .map(|(_, v)| v.trim().to_string())
                            .unwrap_or_default()
                    };
                    let length: usize = header("content-length").parse().unwrap_or(0);
                    while buf.len() < head_end + length {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
                    let (method, path) = (
                        first.next().unwrap_or_default().to_string(),
                        first.next().unwrap_or_default().to_string(),
                    );
                    record.lock().unwrap().push((
                        method,
                        path.clone(),
                        header("authorization"),
                        length,
                    ));
                    let (status, body) = if path.starts_with("/health") {
                        (200, r#"{"version":"1.9.0","apiVersion":"2"}"#.to_string())
                    } else if path.starts_with("/models") {
                        (
                            200,
                            r#"[{"id":"parakeet_v3","name":"Parakeet v3","installed":true,"verified":true}]"#
                                .to_string(),
                        )
                    } else {
                        let next = {
                            let mut q = queue.lock().unwrap();
                            (!q.is_empty()).then(|| q.remove(0))
                        };
                        match next {
                            Some((s, b)) => (s, b.to_string()),
                            None => {
                                let seconds = length.saturating_sub(44) / 2 / SAMPLE_RATE;
                                (
                                    200,
                                    format!(
                                        r#"{{"transcript":" piece of {seconds}s ","durationMs":1}}"#
                                    ),
                                )
                            }
                        }
                    };
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        (port, seen)
    }

    fn discovery(dir: &Path, port: u16) -> PathBuf {
        let path = dir.join("local-api.json");
        std::fs::write(
            &path,
            format!(r#"{{"port":{port},"token":"sekret","apiVersion":"2","pid":1}}"#),
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn a_long_recording_goes_to_hex_in_30_second_pieces_with_the_token() {
        let (port, seen) = fake_hex(vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        let client = Client::connect_at(&discovery(dir.path(), port))
            .await
            .unwrap();
        assert_eq!(client.health().await.unwrap(), "1.9.0");
        assert!(client.models().await.unwrap()[0].usable());
        // 65 seconds: pieces of 30, 30 and 5.
        let samples = vec![0.1f32; SAMPLE_RATE * 65];
        let text = client
            .transcribe(&samples, "parakeet_v3", "pt")
            .await
            .unwrap();
        assert_eq!(text, "piece of 30s piece of 30s piece of 5s");
        let seen = seen.lock().unwrap().clone();
        let posts: Vec<_> = seen.iter().filter(|s| s.0 == "POST").collect();
        assert_eq!(posts.len(), 3);
        assert!(posts.iter().all(|p| p.2 == "Bearer sekret"));
        assert!(posts[0].1.contains("model=parakeet_v3") && posts[0].1.contains("language=pt"));
        assert!(posts
            .iter()
            .all(|p| p.3 <= 44 + SAMPLE_RATE * PIECE_SECONDS * 2));
    }

    #[tokio::test]
    async fn hex_s_refusals_are_named_and_a_busy_queue_is_retried() {
        let (port, _) = fake_hex(vec![
            (429, r#"{"code":"queue-full"}"#),
            (200, r#"{"transcript":"after waiting"}"#),
            (409, r#"{"error":{"code":"model-not-ready"}}"#),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let client = Client::connect_at(&discovery(dir.path(), port))
            .await
            .unwrap();
        let samples = vec![0.0f32; SAMPLE_RATE];
        assert_eq!(
            client.transcribe(&samples, "m", "").await.unwrap(),
            "after waiting"
        );
        let err = client.transcribe(&samples, "m", "").await.unwrap_err();
        // No language chosen: none is sent, and Hex uses its own.
        assert!(
            matches!(&err, Failure::Refused(m) if m.contains("hasn't finished loading")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn without_hex_running_it_is_unavailable_not_an_error_of_the_recording() {
        let dir = tempfile::tempdir().unwrap();
        // No discovery file.
        assert!(matches!(
            Client::connect_at(&dir.path().join("local-api.json")).await,
            Err(Failure::Unavailable(_))
        ));
        // A discovery file whose port answers nothing.
        let closed = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(matches!(
            Client::connect_at(&discovery(dir.path(), closed)).await,
            Err(Failure::Unavailable(_))
        ));
        // Settings says "not running" in its own words; only an odder reason is passed on.
        let closed_hex = status_of(Client::connect_at(&discovery(dir.path(), closed)).await).await;
        assert!(!closed_hex.running && closed_hex.problem.is_none());
        std::fs::write(dir.path().join("local-api.json"), "{}").unwrap();
        let broken = status_of(Client::connect_at(&dir.path().join("local-api.json")).await).await;
        assert!(!broken.running && broken.problem.is_some());
    }
}
