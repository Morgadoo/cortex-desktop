//! Teach a task by recording (OpenBot's): the user does a task once while Cortex watches, and a
//! vision model turns what it saw into a skill, which the user reviews before saving.
//!
//! While recording, Cortex takes a screenshot about every second and keeps it when the screen has
//! changed (or another window came forward), with the active window's title. It never logs keys
//! or what was typed; the screenshots stay in memory and are dropped once the draft is made or
//! the recording is thrown away. A small "Recording" pill stays on top with the time and a Stop
//! button, and the main window steps aside until it is pressed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rig_core::completion::message::{ImageMediaType, Message, UserContent};
use rig_core::completion::CompletionRequest;
use serde::{Deserialize, Serialize};

pub const PILL: &str = "recorder";
/// Screenshots kept at most; when more come, every other middle one goes.
const MAX_FRAMES: usize = 40;
/// Screenshots shown to the model.
const SENT_FRAMES: usize = 12;
/// A recording stops by itself after this long.
const MAX_LENGTH: Duration = Duration::from_secs(15 * 60);
const EVERY: Duration = Duration::from_millis(1200);
/// Share of thumbnail pixels that must change for a new step: a typed command or a new line of
/// output does, a blinking cursor does not.
const CHANGED: f64 = 0.003;
/// How much one pixel must change to count.
const PIXEL: u8 = 24;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub seconds: f64,
    pub window: String,
    /// JPEG, base64, at most 1024 wide.
    #[serde(skip)]
    pub image: String,
}

#[derive(Default)]
pub struct Recorder {
    inner: Mutex<Option<Session>>,
    /// The last finished recording, waiting for its draft.
    done: Mutex<Vec<Frame>>,
    /// How long it lasted, in seconds.
    done_seconds: std::sync::atomic::AtomicU64,
}

struct Session {
    stop: Arc<AtomicBool>,
    /// Set once the loop has taken its last screenshot.
    finished: Arc<AtomicBool>,
    frames: Arc<Mutex<Vec<Frame>>>,
    started: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub recording: bool,
    pub seconds: u64,
    pub steps: usize,
}

/// The title of the window in front, where the system says.
pub(crate) fn active_window() -> String {
    let run = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if cfg!(target_os = "macos") {
        return run("osascript", &["-e", "tell application \"System Events\" to get name of first application process whose frontmost is true"]).unwrap_or_default();
    }
    if cfg!(target_os = "linux") {
        return run("xdotool", &["getactivewindow", "getwindowname"])
            .or_else(|| run("xdotool", &["getwindowfocus", "getwindowname"]))
            .unwrap_or_default();
    }
    String::new()
}

/// A small greyscale copy for telling whether the screen changed.
fn thumbnail(png: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(png).ok()?;
    Some(
        image
            .resize_exact(160, 90, image::imageops::FilterType::Triangle)
            .to_luma8()
            .into_raw(),
    )
}

/// The share of pixels that changed noticeably between two thumbnails (1 when they differ in size).
pub(in crate::chat::multimedia) fn difference(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    a.iter()
        .zip(b)
        .filter(|(x, y)| x.abs_diff(**y) > PIXEL)
        .count() as f64
        / a.len() as f64
}

/// Keeps the first and last frames and thins the middle to stay under the limit.
pub(in crate::chat::multimedia) fn thin<T>(frames: &mut Vec<T>, max: usize) {
    while frames.len() > max {
        let last = frames.len() - 1;
        let mut i = 0;
        frames.retain(|_| {
            let keep = i == 0 || i == last || i % 2 == 0;
            i += 1;
            keep
        });
    }
}

/// Up to `n` frames for the model: evenly spread, always the first and last, and every frame
/// where another window came forward when there is room.
pub(in crate::chat::multimedia) fn pick(frames: &[Frame], n: usize) -> Vec<usize> {
    if frames.len() <= n {
        return (0..frames.len()).collect();
    }
    let mut chosen: Vec<usize> = (0..n).map(|k| k * (frames.len() - 1) / (n - 1)).collect();
    for (i, pair) in frames.windows(2).enumerate() {
        if pair[0].window != pair[1].window
            && !chosen.contains(&(i + 1))
            && chosen.len() < n + n / 2
        {
            chosen.push(i + 1);
        }
    }
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}

impl Recorder {
    pub fn status(&self) -> Status {
        match self.inner.lock().unwrap().as_ref() {
            Some(s) => Status {
                recording: true,
                seconds: s.started.elapsed().as_secs(),
                steps: s.frames.lock().unwrap().len(),
            },
            None => Status {
                recording: false,
                seconds: self.done_seconds.load(Ordering::Relaxed),
                steps: self.done.lock().unwrap().len(),
            },
        }
    }

    pub fn start(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.is_some() {
            return Err("A recording is already running.".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(Mutex::new(Vec::new()));
        *inner = Some(Session {
            stop: stop.clone(),
            finished: finished.clone(),
            frames: frames.clone(),
            started: Instant::now(),
        });
        self.done.lock().unwrap().clear();
        tauri::async_runtime::spawn(async move {
            let started = Instant::now();
            let mut last: Option<(Vec<u8>, String)> = None;
            loop {
                // One last look when stopping: how the task ended matters most.
                let ending = stop.load(Ordering::Relaxed) || started.elapsed() >= MAX_LENGTH;
                if let Ok(png) = crate::chat::computer::capture().await {
                    let window = active_window();
                    if let Some(thumb) = thumbnail(&png) {
                        let changed = last.as_ref().is_none_or(|(t, w)| {
                            *w != window
                                || difference(t, &thumb) > if ending { 0.0 } else { CHANGED }
                        });
                        if changed {
                            if let Ok((image, ..)) = crate::chat::computer::shrink(&png, 1024) {
                                let mut list = frames.lock().unwrap();
                                list.push(Frame {
                                    seconds: started.elapsed().as_secs_f64(),
                                    window: window.clone(),
                                    image,
                                });
                                thin(&mut list, MAX_FRAMES);
                            }
                            last = Some((thumb, window));
                        }
                    }
                }
                if ending {
                    break;
                }
                tokio::time::sleep(EVERY).await;
            }
            finished.store(true, Ordering::Relaxed);
        });
        Ok(())
    }

    /// Stops recording; the frames wait for [`Recorder::draft`].
    pub fn stop(&self) -> Status {
        let session = self.inner.lock().unwrap().take();
        if let Some(session) = session {
            session.stop.store(true, Ordering::Relaxed);
            // Wait (briefly) for the last screenshot.
            let waited = Instant::now();
            while !session.finished.load(Ordering::Relaxed)
                && waited.elapsed() < Duration::from_secs(4)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            self.done_seconds
                .store(session.started.elapsed().as_secs(), Ordering::Relaxed);
            *self.done.lock().unwrap() = std::mem::take(&mut *session.frames.lock().unwrap());
        }
        self.status()
    }

    pub fn discard(&self) {
        self.stop();
        self.done.lock().unwrap().clear();
    }

    pub fn frames(&self) -> Vec<Frame> {
        self.done.lock().unwrap().clone()
    }
}

/// What the model drafted from a recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    pub name: String,
    pub description: String,
    pub body: String,
}

pub(in crate::chat::multimedia) fn parse_draft(reply: &str) -> Result<Draft, String> {
    let start = reply.find('{').ok_or("The model did not return a skill.")?;
    let end = reply
        .rfind('}')
        .ok_or("The model did not return a skill.")?;
    let mut d: Draft = serde_json::from_str(&reply[start..=end]).map_err(|_| {
        "The model's draft could not be read; try again or another model.".to_string()
    })?;
    d.name = d
        .name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if d.name.is_empty() || d.body.trim().is_empty() {
        return Err("The draft was empty; try again.".into());
    }
    Ok(d)
}

/// Asks `model` (one that reads images) to turn the recording into a skill.
pub async fn draft(
    config_dir: &std::path::Path,
    model: &str,
    frames: &[Frame],
    note: &str,
) -> Result<Draft, String> {
    if frames.is_empty() {
        return Err(
            "Nothing was recorded. Record the task again and change something on screen.".into(),
        );
    }
    let chosen = pick(frames, SENT_FRAMES);
    let mut content = vec![UserContent::text(format!(
        "The user recorded themselves doing a task on their computer so that you can learn it. Below are {} \
         screenshots in order, each with the time and the window in front. Ignore Cortex's small \"Recording\" pill.\n\
         {}\n\
         Write a reusable skill for an AI assistant that can see the screen and use the mouse and keyboard \
         (the `computer` tool), web search and files. Describe the goal, what is needed first, then numbered steps \
         naming the apps, menus, buttons and fields as they appear, what to check before moving on, and how to \
         tell it worked. Where a step depends on details that change each time (names, dates, amounts), say to ask \
         the user or take them from the request. Do not include passwords or personal data seen on screen.\n\n\
         Reply with one JSON object and nothing else: {{\"name\": \"short-kebab-case-name\", \"description\": \
         \"when to use this skill, in one sentence\", \"body\": \"the instructions, in Markdown\"}}.",
        chosen.len(),
        if note.trim().is_empty() { String::new() } else { format!("The user's note on what the task is: {}\n", note.trim()) }
    ))];
    for i in chosen {
        let f = &frames[i];
        content.push(UserContent::text(format!(
            "Step at {:.0}s — window: {}",
            f.seconds,
            if f.window.is_empty() {
                "unknown"
            } else {
                &f.window
            }
        )));
        // The image is checked to decode before it goes out.
        if base64::engine::general_purpose::STANDARD
            .decode(&f.image)
            .is_ok()
        {
            content.push(UserContent::image_base64(
                f.image.clone(),
                Some(ImageMediaType::JPEG),
                None,
            ));
        }
    }
    let request = CompletionRequest::new(Message::User { content }).max_tokens(6_000u64);
    let reply = crate::chat::connect::ask_text(config_dir, model, request, 240).await?;
    parse_draft(&reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seconds: f64, window: &str) -> Frame {
        Frame {
            seconds,
            window: window.into(),
            image: String::new(),
        }
    }

    #[test]
    fn changes_are_measured_on_small_copies() {
        assert_eq!(difference(&[10, 10, 10], &[10, 10, 10]), 0.0);
        assert!((difference(&[0, 0, 0, 0], &[40, 0, 0, 0]) - 0.25).abs() < 1e-9);
        assert_eq!(difference(&[0, 0], &[20, 0]), 0.0, "small shifts are noise");
        assert_eq!(difference(&[], &[]), 1.0);
    }

    #[test]
    fn long_recordings_are_thinned_keeping_both_ends() {
        let mut v: Vec<usize> = (0..100).collect();
        thin(&mut v, 40);
        assert!(v.len() <= 40);
        assert_eq!(v.first(), Some(&0));
        assert_eq!(v.last(), Some(&99));
    }

    #[test]
    fn the_model_sees_spread_frames_and_window_changes() {
        let frames: Vec<Frame> = (0..30)
            .map(|i| frame(i as f64, if i < 17 { "Files" } else { "Mail" }))
            .collect();
        let chosen = pick(&frames, 6);
        assert_eq!(chosen.first(), Some(&0));
        assert_eq!(chosen.last(), Some(&29));
        assert!(
            chosen.contains(&17),
            "the switch to Mail is shown: {chosen:?}"
        );
        assert_eq!(pick(&frames[..4], 6), vec![0, 1, 2, 3]);
    }

    #[test]
    fn drafts_are_read_and_named_safely() {
        let d = parse_draft("Sure:\n{\"name\": \"Export Invoice PDF!\", \"description\": \"When asked to export an invoice.\", \"body\": \"1. Open...\"}").unwrap();
        assert_eq!(d.name, "export-invoice-pdf");
        assert!(parse_draft("{\"name\": \"\", \"description\": \"x\", \"body\": \"y\"}").is_err());
        assert!(parse_draft("no").is_err());
    }
}
