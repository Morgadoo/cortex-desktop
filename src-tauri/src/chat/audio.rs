//! Audio settings, set up as Open WebUI does: a speech-to-text engine for voice input and a
//! text-to-speech engine for reading answers aloud, each with its own server, key, model and
//! voice, independent of the chat providers.
//!
//! Engines:
//! - `provider`: the chat providers already set up (rig's OpenAI, Groq or xAI models).
//! - `openai`: any OpenAI-compatible server, through rig's OpenAI-shaped models: OpenAI, Groq,
//!   or a local one (Speaches, LocalAI, Kokoro-FastAPI, faster-whisper-server…).
//! - `xai`: xAI's text-to-speech, through rig's xAI dialect.
//! - `elevenlabs`: ElevenLabs, called directly (rig has no ElevenLabs model).
//! - `hex` (speech only): the Hex dictation app's models on this computer, through its local
//!   API (`hex.rs`); when Hex isn't running, the built-in Whisper takes over.
//! - `browser` (speech only): the system voice, through the page's Web Speech API.
//! - `off`.
//!
//! Keys live in the OS keychain (`audio-stt`, `audio-tts`); the rest in `chat/audio.json`.

use crate::support::settings_file::SettingsFile;
use std::path::Path;

use rig_core::audio_generation::AudioGenerationRequestBuilder;
use rig_core::providers::openai::wire::{self as dialects};
use rig_core::providers::openai::OpenAIConfig;
use rig_core::providers::xai;
use rig_core::transcription::TranscriptionRequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STT_SECRET: &str = "audio-stt";
pub const TTS_SECRET: &str = "audio-tts";
const ELEVENLABS: &str = "https://api.elevenlabs.io";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct SttSettings {
    /// `provider`, `builtin`, `hex`, `openai`, `elevenlabs` or `off`.
    pub engine: String,
    /// For `openai`: the server's base URL, ending in `/v1`.
    pub base_url: String,
    pub model: String,
    /// ISO 639-1 code (`en`, `pt`…); empty lets the model detect it.
    pub language: String,
    /// Send the message as soon as the transcript arrives.
    pub auto_send: bool,
    /// Whether a key is saved in the keychain (never the key itself).
    pub has_key: bool,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            engine: "provider".into(),
            base_url: String::new(),
            model: String::new(),
            language: String::new(),
            auto_send: false,
            has_key: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TtsSettings {
    /// `provider`, `openai`, `xai`, `elevenlabs`, `browser` or `off`.
    pub engine: String,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    /// 0.25–4.0, for engines that take it.
    pub speed: f32,
    /// Read every new answer aloud when it finishes.
    pub auto_play: bool,
    pub has_key: bool,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            engine: "provider".into(),
            base_url: String::new(),
            model: String::new(),
            voice: String::new(),
            speed: 1.0,
            auto_play: false,
            has_key: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub stt: SttSettings,
    pub tts: TtsSettings,
}

const AUDIO_FILE: SettingsFile<AudioSettings> = SettingsFile::sealed("chat/audio.json");

pub fn load(config_dir: &Path) -> AudioSettings {
    AUDIO_FILE.load(config_dir)
}

pub fn save(config_dir: &Path, settings: &AudioSettings) -> Result<(), String> {
    AUDIO_FILE.save(config_dir, settings)
}

/// Whether live transcripts may use this engine: one on this computer, asked again about once a
/// second while the speaker talks (never a paid service). `live.ts` in the page decides the same.
pub fn live_capable(stt: &SttSettings) -> bool {
    match stt.engine.as_str() {
        "builtin" | "hex" => true,
        "openai" => {
            let url = stt.base_url.trim();
            let rest = url
                .strip_prefix("http://")
                .or_else(|| url.strip_prefix("https://"))
                .unwrap_or("");
            ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| {
                rest.strip_prefix(host)
                    .is_some_and(|after| after.is_empty() || after.starts_with([':', '/']))
            })
        }
        _ => false,
    }
}

/// Settings cleaned before saving: known engines, a speed in range, trimmed fields.
pub fn normalize(mut s: AudioSettings) -> AudioSettings {
    if !["provider", "builtin", "hex", "openai", "elevenlabs", "off"]
        .contains(&s.stt.engine.as_str())
    {
        s.stt.engine = "provider".into();
    }
    if !["provider", "openai", "xai", "elevenlabs", "browser", "off"]
        .contains(&s.tts.engine.as_str())
    {
        s.tts.engine = "provider".into();
    }
    for field in [
        &mut s.stt.base_url,
        &mut s.stt.model,
        &mut s.stt.language,
        &mut s.tts.base_url,
        &mut s.tts.model,
        &mut s.tts.voice,
    ] {
        *field = field.trim().to_string();
    }
    s.stt.base_url = s.stt.base_url.trim_end_matches('/').to_string();
    s.tts.base_url = s.tts.base_url.trim_end_matches('/').to_string();
    s.stt.language = s.stt.language.to_lowercase().chars().take(5).collect();
    if !(0.25..=4.0).contains(&s.tts.speed) {
        s.tts.speed = 1.0;
    }
    s
}

/// Voices an engine offers, for the settings page: listed by the service where it can, else
/// the well-known ones.
pub fn known_voices(engine: &str) -> Vec<(String, String)> {
    let names: &[&str] = match engine {
        "openai" | "provider" => &[
            "alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer",
            "verse",
        ],
        "xai" => &["eve", "ara", "rex", "sal", "leo"],
        _ => &[],
    };
    names
        .iter()
        .map(|n| (n.to_string(), n.to_string()))
        .collect()
}

async fn secret(name: &str) -> Result<Option<String>, String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    crate::tools::auth::keychain_secrets(account)
        .load(name)
        .await
}

fn mime_of(audio: &[u8]) -> &'static str {
    match audio {
        [b'R', b'I', b'F', b'F', ..] => "audio/wav",
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        [b'f', b'L', b'a', b'C', ..] => "audio/flac",
        _ => "audio/mpeg",
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_default()
}

/// Reads `text` (already prose) aloud with the chosen engine. `Ok(None)` means the page speaks it
/// itself (the `browser` engine).
pub async fn speak(
    config_dir: &Path,
    text: &str,
    settings: &TtsSettings,
) -> Result<Option<(Vec<u8>, String)>, String> {
    let speed = (settings.speed - 1.0).abs() > f32::EPSILON;
    let audio = match settings.engine.as_str() {
        "off" => return Err("Read aloud is off in Settings → Audio.".into()),
        "browser" => return Ok(None),
        "provider" => return super::media::speak(config_dir, text).await.map(Some),
        "openai" | "xai" => {
            let key = secret(TTS_SECRET).await?.unwrap_or_default();
            let xai = settings.engine == "xai";
            let mut config = OpenAIConfig::with_key(
                if xai {
                    &xai::DIALECT
                } else {
                    &dialects::OPENAI
                },
                key,
            );
            if !settings.base_url.is_empty() {
                config = config.with_base_url(if xai {
                    super::providers::normalize_base_url("xai", &settings.base_url)
                } else {
                    settings.base_url.clone()
                });
            } else if !xai {
                return Err("Set the text-to-speech server's base URL in Settings → Audio.".into());
            }
            let model = if settings.model.is_empty() {
                if xai {
                    "grok-tts".into()
                } else {
                    "tts-1".into()
                }
            } else {
                settings.model.clone()
            };
            let voice = if settings.voice.is_empty() {
                if xai {
                    "eve"
                } else {
                    "alloy"
                }
            } else {
                settings.voice.as_str()
            };
            let mut request = AudioGenerationRequestBuilder::new(text, voice);
            if speed && !xai {
                request = request.speed(settings.speed);
            }
            config
                .client()
                .audio_generation(model)
                .call(request.build())
                .await
                .map_err(|e| e.to_string())?
                .audio
        }
        "elevenlabs" => {
            let key = secret(TTS_SECRET)
                .await?
                .ok_or("Add your ElevenLabs API key in Settings → Audio.")?;
            if settings.voice.is_empty() {
                return Err("Choose an ElevenLabs voice in Settings → Audio.".into());
            }
            let base = if settings.base_url.is_empty() {
                ELEVENLABS
            } else {
                settings.base_url.as_str()
            };
            let model = if settings.model.is_empty() {
                "eleven_multilingual_v2"
            } else {
                settings.model.as_str()
            };
            let mut body = json!({ "text": text, "model_id": model });
            if speed {
                body["voice_settings"] = json!({ "speed": settings.speed.clamp(0.7, 1.2) });
            }
            let response = http()
                .post(format!(
                    "{base}/v1/text-to-speech/{}?output_format=mp3_44100_128",
                    settings.voice
                ))
                .header("xi-api-key", key)
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(format!(
                    "ElevenLabs: {}",
                    response
                        .text()
                        .await
                        .unwrap_or_default()
                        .chars()
                        .take(300)
                        .collect::<String>()
                ));
            }
            response.bytes().await.map_err(|e| e.to_string())?.to_vec()
        }
        other => return Err(format!("Unknown text-to-speech engine `{other}`.")),
    };
    if audio.is_empty() {
        return Err("The voice service sent back no audio.".into());
    }
    let mime = mime_of(&audio).to_string();
    Ok(Some((audio, mime)))
}

/// Transcribes recorded speech with the chosen engine. `live` marks a guess made while the
/// speaker is still talking (`live.ts` in the page), which the built-in Whisper answers faster.
pub async fn transcribe(
    config_dir: &Path,
    audio: Vec<u8>,
    file_name: &str,
    settings: &SttSettings,
    live: bool,
) -> Result<String, String> {
    match settings.engine.as_str() {
        "off" => Err("Voice input is off in Settings → Audio.".into()),
        "provider" => super::media::transcribe(config_dir, audio, file_name).await,
        "builtin" => builtin(audio, &settings.model, &settings.language, live).await,
        "hex" => {
            let samples = {
                let audio = audio.clone();
                tokio::task::spawn_blocking(move || super::local_models::audio_samples(&audio))
                    .await
                    .map_err(|e| e.to_string())??
            };
            match with_hex(&samples, settings).await {
                Ok(text) => Ok(text),
                // Hex isn't running: the built-in Whisper takes over, when one is downloaded.
                Err(super::hex::Failure::Unavailable(why)) => {
                    log::info!("speech-to-text: {why} Using the built-in Whisper instead.");
                    builtin(audio, "", &settings.language, live).await.map_err(|e| {
                        if e == NO_WHISPER {
                            format!(
                                "{why} Start Hex, or download a Whisper model (Settings → Providers → Built-in models) for when it isn't running."
                            )
                        } else {
                            e
                        }
                    })
                }
                Err(super::hex::Failure::Refused(why)) => Err(why),
            }
        }
        "openai" => {
            if settings.base_url.is_empty() {
                return Err("Set the speech-to-text server's base URL in Settings → Audio.".into());
            }
            let key = secret(STT_SECRET).await?.unwrap_or_default();
            let config = OpenAIConfig::with_key(&dialects::OPENAI, key)
                .with_base_url(settings.base_url.clone());
            let model = if settings.model.is_empty() {
                "whisper-1".into()
            } else {
                settings.model.clone()
            };
            let mut request =
                TranscriptionRequestBuilder::new(audio).filename(Some(file_name.to_string()));
            if !settings.language.is_empty() {
                request = request.language(settings.language.clone());
            }
            let response = config
                .client()
                .transcription(model)
                .call(request.build())
                .await
                .map_err(|e| e.to_string())?;
            Ok(response.text.trim().to_string())
        }
        "elevenlabs" => {
            let key = secret(STT_SECRET)
                .await?
                .ok_or("Add your ElevenLabs API key in Settings → Audio.")?;
            let base = if settings.base_url.is_empty() {
                ELEVENLABS
            } else {
                settings.base_url.as_str()
            };
            let model = if settings.model.is_empty() {
                "scribe_v1"
            } else {
                settings.model.as_str()
            };
            let mut form = reqwest::multipart::Form::new()
                .text("model_id", model.to_string())
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(audio).file_name(file_name.to_string()),
                );
            if !settings.language.is_empty() {
                form = form.text("language_code", settings.language.clone());
            }
            let response = http()
                .post(format!("{base}/v1/speech-to-text"))
                .header("xi-api-key", key)
                .multipart(form)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status();
            let body: Value = response.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!(
                    "ElevenLabs: {}",
                    body.to_string().chars().take(300).collect::<String>()
                ));
            }
            Ok(body
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string())
        }
        other => Err(format!("Unknown speech-to-text engine `{other}`.")),
    }
}

const NO_WHISPER: &str = "Download a Whisper model first (Settings → Providers → Built-in models).";

/// The built-in Whisper: the model `chosen` when it is downloaded, else the first one that is.
async fn builtin(
    audio: Vec<u8>,
    chosen: &str,
    language: &str,
    live: bool,
) -> Result<String, String> {
    let root = super::local_models::root()?;
    let speech = |id: &str| {
        super::local_models::entry(id).is_some_and(|e| e.kind == super::local_models::Kind::Speech)
            && super::local_models::installed(&root, id)
    };
    let id = Some(chosen.to_string())
        .filter(|m| speech(m))
        .or_else(|| {
            super::local_models::list(&root)
                .into_iter()
                .find(|m| m.installed && m.kind == super::local_models::Kind::Speech)
                .map(|m| m.id)
        })
        .ok_or(NO_WHISPER)?;
    let language = language.to_string();
    tokio::task::spawn_blocking(move || {
        let samples = super::local_models::audio_samples(&audio)?;
        super::local_models::transcribe(&root, &id, &samples, &language, live)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Hex, with the model chosen in Settings when it is ready there, else its best ready one.
async fn with_hex(samples: &[f32], settings: &SttSettings) -> Result<String, super::hex::Failure> {
    let client = super::hex::Client::connect().await?;
    let models = client.models().await?;
    let model = super::hex::pick_model(&models, &settings.model).ok_or_else(|| {
        super::hex::Failure::Refused(
            "Hex has no speech model downloaded yet; download one in Hex.".into(),
        )
    })?;
    client.transcribe(samples, &model, &settings.language).await
}

/// ElevenLabs voices on the user's account, as (id, name).
pub async fn elevenlabs_voices(base_url: &str) -> Result<Vec<(String, String)>, String> {
    let key = secret(TTS_SECRET)
        .await?
        .ok_or("Add your ElevenLabs API key first.")?;
    let base = if base_url.is_empty() {
        ELEVENLABS
    } else {
        base_url
    };
    let body: Value = http()
        .get(format!("{base}/v1/voices"))
        .header("xi-api-key", key)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(body
        .get("voices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some((
                v.get("voice_id")?.as_str()?.to_string(),
                v.get("name")?.as_str()?.to_string(),
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_transcripts_use_only_engines_on_this_computer() {
        let stt = |engine: &str, base_url: &str| SttSettings {
            engine: engine.into(),
            base_url: base_url.into(),
            ..SttSettings::default()
        };
        assert!(live_capable(&stt("builtin", "")) && live_capable(&stt("hex", "")));
        assert!(live_capable(&stt("openai", "http://localhost:8000/v1")));
        assert!(live_capable(&stt("openai", "http://127.0.0.1")));
        assert!(live_capable(&stt("openai", "http://[::1]:9000/v1")));
        assert!(!live_capable(&stt("openai", "https://api.openai.com/v1")));
        assert!(!live_capable(&stt(
            "openai",
            "http://localhost.evil.example/v1"
        )));
        assert!(!live_capable(&stt("provider", "")) && !live_capable(&stt("elevenlabs", "")));
    }

    #[test]
    fn settings_are_normalized_and_saved() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), AudioSettings::default());
        let mut s = AudioSettings::default();
        s.stt.engine = "whisper-local".into();
        s.stt.base_url = " http://localhost:8000/v1/ ".into();
        s.stt.language = "PT".into();
        s.tts.engine = "elevenlabs".into();
        s.tts.speed = 9.0;
        let s = normalize(s);
        let mut hex = AudioSettings::default();
        hex.stt.engine = "hex".into();
        assert_eq!(
            normalize(hex).stt.engine,
            "hex",
            "Hex is a speech-to-text engine"
        );
        assert_eq!(s.stt.engine, "provider", "unknown engines fall back");
        assert_eq!(s.stt.base_url, "http://localhost:8000/v1");
        assert_eq!(s.stt.language, "pt");
        assert_eq!(s.tts.engine, "elevenlabs");
        assert_eq!(s.tts.speed, 1.0);
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
        assert!(known_voices("openai").iter().any(|(id, _)| id == "nova"));
        assert_eq!(mime_of(b"RIFF...."), "audio/wav");
    }
}
