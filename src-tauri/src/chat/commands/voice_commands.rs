//! Voice: speech in and out, and the wake word.

use super::*;

/// Reads text aloud; returns base64 audio and its media type.
#[tauri::command]
pub async fn chat_speak(app: AppHandle, text: String) -> Result<SpeechOut, String> {
    use base64::Engine as _;
    let dir = config_dir(&app)?;
    let settings = crate::chat::audio::load(&dir).tts;
    let prose = crate::chat::media::speakable(&text);
    if prose.is_empty() {
        return Err("There is nothing to read.".into());
    }
    Ok(
        match crate::chat::audio::speak(&dir, &prose, &settings).await? {
            Some((audio, mime)) => SpeechOut::Audio {
                data: base64::engine::general_purpose::STANDARD.encode(audio),
                mime,
            },
            // The page speaks it with the system voice.
            None => SpeechOut::Browser {
                text: prose,
                rate: settings.speed,
                voice: settings.voice,
            },
        },
    )
}

#[tauri::command]
pub fn chat_audio_settings(app: AppHandle) -> Result<AudioView, String> {
    let dir = config_dir(&app)?;
    let file = crate::chat::providers::load(&dir);
    Ok(AudioView {
        settings: crate::chat::audio::load(&dir),
        provider_speech_model: crate::chat::media::speech_model(&file),
        provider_voice_model: crate::chat::media::voice_model(&file),
    })
}

/// Saves the audio settings; a key is stored in the keychain when given, removed when cleared.
#[tauri::command]
pub async fn chat_save_audio_settings(
    app: AppHandle,
    settings: crate::chat::audio::AudioSettings,
    stt_key: Option<String>,
    tts_key: Option<String>,
    clear_stt_key: bool,
    clear_tts_key: bool,
) -> Result<AudioView, String> {
    let dir = config_dir(&app)?;
    let previous = crate::chat::audio::load(&dir);
    let mut settings = crate::chat::audio::normalize(settings);
    settings.stt.has_key = previous.stt.has_key;
    settings.tts.has_key = previous.tts.has_key;
    for (key, clear, name, has) in [
        (
            stt_key,
            clear_stt_key,
            crate::chat::audio::STT_SECRET,
            &mut settings.stt.has_key,
        ),
        (
            tts_key,
            clear_tts_key,
            crate::chat::audio::TTS_SECRET,
            &mut settings.tts.has_key,
        ),
    ] {
        if clear {
            secrets()?.delete(name).await?;
            *has = false;
        }
        if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            secrets()?.store(name, &key).await?;
            *has = true;
        }
    }
    crate::chat::audio::save(&dir, &settings)?;
    chat_audio_settings(app)
}

/// Voices for a text-to-speech engine: the account's voices for ElevenLabs, the known ones else.
#[tauri::command]
pub async fn chat_audio_voices(
    engine: String,
    base_url: String,
) -> Result<Vec<(String, String)>, String> {
    if engine == "elevenlabs" {
        return crate::chat::audio::elevenlabs_voices(base_url.trim().trim_end_matches('/')).await;
    }
    Ok(crate::chat::audio::known_voices(&engine))
}
