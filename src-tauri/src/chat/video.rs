//! Video attachments: a video is read as what a model can take, spread-out still frames (for
//! models that see images) and the soundtrack's transcript (with the speech-to-text set in
//! Settings → Audio). Uses ffmpeg and ffprobe when they are installed.

use std::path::Path;

use base64::Engine as _;
use serde::Serialize;

/// Frames taken from a video.
const FRAMES: usize = 6;
const FRAME_WIDTH: u32 = 768;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// Seconds into the video.
    pub at: f64,
    /// JPEG, base64.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    pub seconds: f64,
    pub frames: Vec<Frame>,
    /// What was said, when the video has sound and speech-to-text is set up.
    pub transcript: Option<String>,
    /// Why there is no transcript, when there is none.
    pub note: Option<String>,
}

/// Times to take frames at: spread over the video, avoiding the very first and last moments
/// (often black).
pub fn frame_times(seconds: f64, n: usize) -> Vec<f64> {
    if seconds <= 0.0 || n == 0 {
        return vec![0.0];
    }
    (0..n)
        .map(|i| seconds * (i as f64 + 0.5) / n as f64)
        .map(|t| (t * 100.0).round() / 100.0)
        .collect()
}

fn run(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|_| format!("{program} is not installed; install ffmpeg to attach videos."))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "{program} could not read the video: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or_default()
        ))
    }
}

/// The video's length and whether it has sound.
fn probe(path: &Path) -> Result<(f64, bool), String> {
    let p = path.to_string_lossy();
    let duration = String::from_utf8_lossy(&run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            &p,
        ],
    )?)
    .trim()
    .parse::<f64>()
    .unwrap_or(0.0);
    let audio = !String::from_utf8_lossy(&run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
            &p,
        ],
    )?)
    .trim()
    .is_empty();
    Ok((duration, audio))
}

/// Frames and transcript of the video at `path`.
pub async fn digest(config_dir: &Path, path: &Path) -> Result<Digest, String> {
    let file = path.to_path_buf();
    let (seconds, has_audio, frames) =
        tokio::task::spawn_blocking(move || -> Result<(f64, bool, Vec<Frame>), String> {
            let (seconds, has_audio) = probe(&file)?;
            let p = file.to_string_lossy().into_owned();
            let scale = format!("scale='min({FRAME_WIDTH},iw)':-2");
            let mut frames = Vec::new();
            for at in frame_times(seconds, FRAMES) {
                let t = format!("{at}");
                let jpeg = run(
                    "ffmpeg",
                    &[
                        "-v",
                        "error",
                        "-ss",
                        &t,
                        "-i",
                        &p,
                        "-frames:v",
                        "1",
                        "-vf",
                        &scale,
                        "-q:v",
                        "4",
                        "-f",
                        "image2",
                        "-c:v",
                        "mjpeg",
                        "pipe:1",
                    ],
                )?;
                if !jpeg.is_empty() {
                    frames.push(Frame {
                        at,
                        data: base64::engine::general_purpose::STANDARD.encode(jpeg),
                    });
                }
            }
            Ok((seconds, has_audio, frames))
        })
        .await
        .map_err(|e| e.to_string())??;
    if frames.is_empty() {
        return Err("No picture could be read from the video.".into());
    }
    let (transcript, note) = if !has_audio {
        (None, Some("The video has no sound.".to_string()))
    } else {
        let p = path.to_string_lossy().into_owned();
        let wav = tokio::task::spawn_blocking(move || {
            run(
                "ffmpeg",
                &[
                    "-v", "error", "-i", &p, "-vn", "-ac", "1", "-ar", "16000", "-f", "wav",
                    "pipe:1",
                ],
            )
        })
        .await
        .map_err(|e| e.to_string())??;
        let settings = super::audio::load(config_dir).stt;
        match super::audio::transcribe(config_dir, wav, "soundtrack.wav", &settings).await {
            Ok(t) if !t.trim().is_empty() => (Some(t), None),
            Ok(_) => (
                None,
                Some("No speech was heard in the soundtrack.".to_string()),
            ),
            Err(e) => (
                None,
                Some(format!("The soundtrack could not be transcribed: {e}")),
            ),
        }
    };
    Ok(Digest {
        seconds,
        frames,
        transcript,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_spread_over_the_video() {
        assert_eq!(
            frame_times(60.0, 6),
            vec![5.0, 15.0, 25.0, 35.0, 45.0, 55.0]
        );
        assert_eq!(frame_times(0.0, 6), vec![0.0]);
        assert_eq!(frame_times(1.0, 2), vec![0.25, 0.75]);
    }

    #[tokio::test]
    async fn a_silent_video_gives_frames_and_says_it_has_no_sound() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            return; // ffmpeg is not installed here
        }
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=3:size=320x240:rate=10",
                "-pix_fmt",
                "yuv420p",
                video.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(made.success());
        let d = digest(dir.path(), &video).await.unwrap();
        assert!((d.seconds - 3.0).abs() < 0.2, "{}", d.seconds);
        assert_eq!(d.frames.len(), FRAMES);
        let first = base64::engine::general_purpose::STANDARD
            .decode(&d.frames[0].data)
            .unwrap();
        assert_eq!(&first[..2], &[0xFF, 0xD8], "a JPEG");
        assert_eq!(
            (d.transcript, d.note.as_deref()),
            (None, Some("The video has no sound."))
        );
    }
}
