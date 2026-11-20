//! Speech to text with Whisper: audio decoding and resampling, mel filters, and transcription.

use super::*;
use crate::chat::error::ChatError;

// Speech: Whisper

/// Whisper's language codes, as its `<|xx|>` tokens.
pub(super) const LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su", "yue",
];

/// The mel filter bank Whisper was trained with (librosa's Slaney-style filters), `n_mels`
/// rows of `n_fft / 2 + 1` weights.
pub(in crate::chat::models) fn mel_filters(n_mels: usize) -> Vec<f32> {
    let sr = 16_000.0f64;
    let n_fft = 400usize;
    let bins = n_fft / 2 + 1;
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    let hz_to_mel = |f: f64| {
        if f < min_log_hz {
            f / f_sp
        } else {
            min_log_mel + (f / min_log_hz).ln() / logstep
        }
    };
    let mel_to_hz = |m: f64| {
        if m < min_log_mel {
            m * f_sp
        } else {
            min_log_hz * (logstep * (m - min_log_mel)).exp()
        }
    };
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let fft_f: Vec<f64> = (0..bins)
        .map(|i| sr / 2.0 * i as f64 / (bins - 1) as f64)
        .collect();
    let mut out = vec![0f32; n_mels * bins];
    for m in 0..n_mels {
        let enorm = 2.0 / (mel_f[m + 2] - mel_f[m]);
        for (k, f) in fft_f.iter().enumerate() {
            let lower = (f - mel_f[m]) / (mel_f[m + 1] - mel_f[m]);
            let upper = (mel_f[m + 2] - f) / (mel_f[m + 2] - mel_f[m + 1]);
            out[m * bins + k] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    out
}

/// Mono 16 kHz samples from a WAV file (16-bit, 24-bit, 32-bit or float), mixed and resampled.
pub(in crate::chat::models) fn wav_samples(bytes: &[u8]) -> Result<Vec<f32>, ChatError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(ChatError::Invalid("not a WAV file".into()));
    }
    let (mut channels, mut rate, mut bits, mut format) = (1usize, 16_000usize, 16usize, 1u16);
    let mut data: &[u8] = &[];
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len =
            u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        let body = &bytes[i + 8..(i + 8 + len).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            format = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]).max(1) as usize;
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
            bits = u16::from_le_bytes([body[14], body[15]]) as usize;
            if format == 0xFFFE && body.len() >= 26 {
                format = u16::from_le_bytes([body[24], body[25]]);
            }
        } else if id == b"data" {
            data = body;
        }
        i += 8 + len + (len & 1);
    }
    let width = bits / 8;
    if width == 0 || data.is_empty() {
        return Err(ChatError::Invalid("The recording is empty.".into()));
    }
    let frames = data.len() / (width * channels);
    let sample = |j: usize| -> f32 {
        let b = &data[j * width..j * width + width];
        match (format, bits) {
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (_, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            (_, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (_, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            (_, 8) => (b[0] as f32 - 128.0) / 128.0,
            _ => 0.0,
        }
    };
    let mono: Vec<f32> = (0..frames)
        .map(|f| (0..channels).map(|c| sample(f * channels + c)).sum::<f32>() / channels as f32)
        .collect();
    Ok(resample(&mono, rate, 16_000))
}

/// Linear resampling, enough for speech.
pub(in crate::chat::models) fn resample(samples: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let n = (samples.len() as f64 * to as f64 / from as f64) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let j = pos as usize;
            let t = (pos - j as f64) as f32;
            let a = samples[j.min(samples.len() - 1)];
            let b = samples[(j + 1).min(samples.len() - 1)];
            a + (b - a) * t
        })
        .collect()
}

/// Samples from any recording: WAV directly, other formats through ffmpeg when installed.
pub(in crate::chat) fn audio_samples(bytes: &[u8]) -> Result<Vec<f32>, ChatError> {
    if let Ok(s) = wav_samples(bytes) {
        return Ok(s);
    }
    let input =
        std::env::temp_dir().join(format!("cortex-audio-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&input, bytes).map_err(ChatError::internal)?;
    let out = std::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i"])
        .arg(&input)
        .args(["-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
        .output();
    let _ = std::fs::remove_file(&input);
    let out = out
        .map_err(|_| {
            "The built-in speech model reads WAV recordings; install ffmpeg for other formats."
                .to_string()
        })
        .map_err(ChatError::Invalid)?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(ChatError::Invalid(
            "The recording could not be read.".into(),
        ));
    }
    Ok(out
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

pub(super) struct Speech {
    pub(super) id: String,
    pub(super) model: candle_transformers::models::whisper::model::Whisper,
    pub(super) tokenizer: tokenizers::Tokenizer,
    pub(super) filters: Vec<f32>,
}

pub(super) fn speech() -> &'static Mutex<Option<Speech>> {
    &super::LOCAL_MODELS.speech
}

/// Audio past the end of the speech that a live guess still reads: a second (100 frames make a
/// second); with less, Whisper repeats itself more.
pub(super) const LIVE_MARGIN_FRAMES: usize = 100;

/// Transcribes 16 kHz mono samples with a downloaded Whisper model. `language` is a code like
/// `en`, or empty to let the model tell. `live` is for a guess while the speaker is still
/// talking: Whisper reads only the audio there is (and half a second more) instead of a
/// 30-second window padded with silence, which is many times less work for a short recording
/// and a little less accurate (a repeated phrase is said once, see [`collapse_repeats`]); the
/// final transcript reads whole windows.
pub(in crate::chat) fn transcribe(
    root: &Path,
    id: &str,
    samples: &[f32],
    language: &str,
    live: bool,
) -> Result<String, ChatError> {
    use candle_core::IndexOp as _;
    use candle_transformers::models::whisper as w;
    if !installed(root, id) {
        return Err(ChatError::Invalid(format!(
            "Download {} first (Settings → Providers → Built-in models).",
            entry(id).map_or(id, |e| e.name)
        )));
    }
    if samples.len() < w::SAMPLE_RATE / 10 {
        return Ok(String::new());
    }
    let mut guard = speech().lock().map_err(ChatError::internal)?;
    if guard.as_ref().is_none_or(|s| s.id != id) {
        *guard = None;
        *guard = Some(load_speech(root, id)?);
    }
    let s = guard
        .as_mut()
        .ok_or_else(|| ChatError::Invalid("The speech model is not loaded.".into()))?;
    let token = |t: &str| {
        s.tokenizer
            .token_to_id(t)
            .ok_or_else(|| format!("The tokenizer has no {t}."))
    };
    let (sot, transcribe, no_ts, eot) = (
        token(w::SOT_TOKEN).map_err(ChatError::Invalid)?,
        token(w::TRANSCRIBE_TOKEN).map_err(ChatError::Invalid)?,
        token(w::NO_TIMESTAMPS_TOKEN).map_err(ChatError::Invalid)?,
        token(w::EOT_TOKEN).map_err(ChatError::Invalid)?,
    );
    let n_mels = s.model.config.num_mel_bins;
    let mel = w::audio::pcm_to_mel(&s.model.config, samples, &s.filters);
    let mel = Tensor::from_vec(mel.clone(), (1, n_mels, mel.len() / n_mels), &Device::Cpu)
        .map_err(ChatError::internal)?;
    let frames = samples.len() / w::HOP_LENGTH;
    let vocab = s.model.config.vocab_size;
    // Only words and the end: no timestamps or other special tokens.
    let mut mask = vec![0f32; vocab];
    for (i, m) in mask.iter_mut().enumerate() {
        if (i as u32 > eot) || s.model.config.suppress_tokens.contains(&(i as u32)) {
            *m = f32::NEG_INFINITY;
        }
    }
    let mask = Tensor::new(mask.as_slice(), &Device::Cpu).map_err(ChatError::internal)?;
    let mut text = String::new();
    let mut seek = 0;
    let max_tokens = s.model.config.max_target_positions / 2;
    // Whisper reads 30-second windows; the mel is padded with silence, so the last one is full too.
    let (_, _, padded) = mel.dims3().map_err(ChatError::internal)?;
    while seek < frames {
        let size = if live {
            // Even: the encoder halves the frames with a stride of two.
            let size = (frames - seek + LIVE_MARGIN_FRAMES)
                .min(padded - seek)
                .min(w::N_FRAMES);
            size - size % 2
        } else {
            (padded - seek).min(w::N_FRAMES)
        };
        let segment = mel.narrow(2, seek, size).map_err(ChatError::internal)?;
        seek += w::N_FRAMES;
        let features = s
            .model
            .encoder
            .forward(&segment, true)
            .map_err(ChatError::internal)?;
        // The language: the one asked for, else the one the model finds likeliest.
        let lang = match language.trim() {
            l if !l.is_empty() => s.tokenizer.token_to_id(&format!("<|{l}|>")),
            _ => likeliest_language(s, &features, sot)?,
        };
        let mut tokens = vec![sot];
        tokens.extend(lang);
        tokens.push(transcribe);
        tokens.push(no_ts);
        let start = tokens.len();
        for i in 0..max_tokens {
            let input = Tensor::new(tokens.as_slice(), &Device::Cpu)
                .and_then(|t| t.unsqueeze(0))
                .map_err(ChatError::internal)?;
            let ys = s
                .model
                .decoder
                .forward(&input, &features, i == 0)
                .map_err(ChatError::internal)?;
            let (_, len, _) = ys.dims3().map_err(ChatError::internal)?;
            let logits = s
                .model
                .decoder
                .final_linear(&ys.i((..1, len - 1..)).map_err(ChatError::internal)?)
                .and_then(|l| l.i(0)?.i(0))
                .map_err(ChatError::internal)?;
            let logits = logits.broadcast_add(&mask).map_err(ChatError::internal)?;
            let next = logits
                .argmax(0)
                .and_then(|t| t.to_scalar::<u32>())
                .map_err(ChatError::internal)?;
            if next == eot {
                break;
            }
            tokens.push(next);
            // A small model can loop on a phrase: stop when the last stretch repeats the one before.
            if is_looping(&tokens[start..]) {
                break;
            }
        }
        let piece = s
            .tokenizer
            .decode(&tokens[start..], true)
            .map_err(ChatError::internal)?;
        let piece = piece.trim();
        if !piece.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece);
        }
    }
    s.model.reset_kv_cache();
    Ok(if live { collapse_repeats(&text) } else { text })
}

/// Loads a downloaded Whisper model, its tokenizer and mel filters.
fn load_speech(root: &Path, id: &str) -> Result<Speech, ChatError> {
    use candle_transformers::models::whisper as w;
    let dir = model_dir(root, id)?;
    let config: w::Config = serde_json::from_str(
        &std::fs::read_to_string(dir.join("config.json")).map_err(ChatError::internal)?,
    )
    .map_err(ChatError::internal)?;
    // SAFETY: the file is ours, downloaded and not changed while it is mapped.
    let vb = unsafe {
        candle_nn::VarBuilder::from_mmaped_safetensors(
            &[dir.join("model.safetensors")],
            w::DTYPE,
            &Device::Cpu,
        )
    }
    .map_err(ChatError::internal)?;
    let filters = mel_filters(config.num_mel_bins);
    let model = w::model::Whisper::load(&vb, config).map_err(ChatError::internal)?;
    let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
        .map_err(ChatError::internal)?;
    Ok(Speech {
        id: id.to_string(),
        model,
        tokenizer,
        filters,
    })
}

/// The language the model finds likeliest for a window of audio, from those it knows.
fn likeliest_language(
    s: &mut Speech,
    features: &Tensor,
    sot: u32,
) -> Result<Option<u32>, ChatError> {
    use candle_core::IndexOp as _;
    let ids: Vec<(u32, &str)> = LANGUAGES
        .iter()
        .filter_map(|l| {
            s.tokenizer
                .token_to_id(&format!("<|{l}|>"))
                .map(|t| (t, *l))
        })
        .collect();
    if ids.is_empty() {
        return Ok(None);
    }
    let input = Tensor::new(&[[sot]], &Device::Cpu).map_err(ChatError::internal)?;
    let ys = s
        .model
        .decoder
        .forward(&input, features, true)
        .map_err(ChatError::internal)?;
    let logits = s
        .model
        .decoder
        .final_linear(&ys.i((..1, 0..1)).map_err(ChatError::internal)?)
        .and_then(|l| l.i(0)?.i(0))
        .map_err(ChatError::internal)?;
    let logits: Vec<f32> = logits.to_vec1().map_err(ChatError::internal)?;
    Ok(ids
        .iter()
        .max_by(|a, b| logits[a.0 as usize].total_cmp(&logits[b.0 as usize]))
        .map(|(t, _)| *t))
}

/// Whether the last stretch of tokens repeats the two before it, as a small model looping on a
/// phrase does.
fn is_looping(said: &[u32]) -> bool {
    (4..=24).any(|n| {
        said.len() >= 3 * n
            && said[said.len() - n..] == said[said.len() - 2 * n..said.len() - n]
            && said[said.len() - 2 * n..said.len() - n]
                == said[said.len() - 3 * n..said.len() - 2 * n]
    })
}

/// A live guess with a phrase said again and again ("Remind me Remind me Remind me"), which
/// Whisper writes when it reads a second of audio, said once. The final transcript keeps
/// everything as heard.
pub(in crate::chat::models) fn collapse_repeats(text: &str) -> String {
    let key = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let mut words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let repeat = (1..=8).find(|&n| {
            i + 2 * n <= words.len() && (0..n).all(|k| key(words[i + k]) == key(words[i + n + k]))
        });
        match repeat {
            Some(n) => {
                words.drain(i + n..i + 2 * n);
            }
            None => i += 1,
        }
    }
    words.join(" ")
}
