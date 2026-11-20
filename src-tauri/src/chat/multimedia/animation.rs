//! The `make_animation` tool: animated videos written as code and rendered on this computer, for
//! explainers, infographics, kinetic titles, charts, product films and slides that move.
//!
//! Two engines, each an open-source program Cortex runs (fetched on first use, never bundled):
//!
//! - **HyperFrames** (HeyGen, Apache-2.0): the composition is an HTML page animated with GSAP,
//!   CSS or any web animation library; `npx hyperframes render` captures it frame by frame in
//!   headless Chrome and encodes it with FFmpeg. Needs Node.js 22 or later.
//! - **ManimGX** (Academa, MIT; native builds GPL): a Python scene in the Manim Community API, for
//!   maths, diagrams and 3D; `manimgx render` draws it on the GPU (or the CPU). Needs `manimgx`
//!   (`uv tool install manimgx`).
//!
//! The source is kept in the chat's workspace (`animations/<name>/`), so a later call can change
//! it; the video goes to `videos/`, where the chat plays it.

use crate::chat::error::ChatError;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use cortex_kernel::tool::{ChatTool, Outcome};

pub const MAKE_ANIMATION: &str = "make_animation";
/// The HyperFrames release used; pinned so a render behaves the same from day to day.
pub const HYPERFRAMES: &str = "hyperframes@0.9.66";
/// The longest a render may take.
const RENDER_LIMIT: Duration = Duration::from_secs(20 * 60);

fn locate(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|d| {
            [
                d.join(program),
                d.join(format!("{program}.exe")),
                d.join(format!("{program}.cmd")),
            ]
        })
        .find(|p| p.is_file())
}

/// Which engines can run here.
pub(in crate::chat::multimedia) fn engines() -> Vec<&'static str> {
    let mut out = Vec::new();
    if locate("npx").is_some() {
        out.push("html");
    }
    if locate("manimgx").is_some() {
        out.push("manim");
    }
    out
}

pub fn definition() -> ChatTool {
    let available = engines();
    let note = if available.is_empty() {
        " Neither engine is installed yet: HyperFrames needs Node.js 22+, ManimGX needs `uv tool install manimgx`; the call says what to install.".to_string()
    } else {
        format!(
            " Installed: {}.",
            available
                .iter()
                .map(|e| if *e == "html" {
                    "html (HyperFrames)"
                } else {
                    "manim (ManimGX)"
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    ChatTool {
        name: MAKE_ANIMATION.into(),
        description: format!(
            "Make an animated video from code: explainers, infographics, kinetic titles, animated \
             charts, product films, slides that move. The video is shown in the chat and saved in \
             the workspace.\n\n\
             engine \"html\" (HyperFrames): `source` is one HTML page, 1920×1080 unless asked. \
             Rules: the root element has id=\"root\", data-composition-id=\"main\", data-start=\"0\", \
             data-duration=\"<seconds>\", data-width, data-height; each element that appears and \
             disappears has class=\"clip\" with data-start, data-duration and data-track-index. \
             Animate with GSAP (load https://cdn.jsdelivr.net/npm/gsap@3.15.0/dist/gsap.min.js): \
             build `const tl = gsap.timeline({{ paused: true }})`, add tweens at absolute times, then \
             `window.__timelines[\"main\"] = tl;`. No video autoplay, no Math.random or Date-based \
             motion (renders must be repeatable). Inline SVG, CSS and web fonts work.\n\n\
             engine \"manim\" (ManimGX): `source` is a Python file `import manimgx as m` with one \
             `class Main(m.Scene)` (or m.ThreeDScene) whose `construct` uses self.play / self.wait; \
             keep content within x∈[-7,7], y∈[-4,4]. Best for maths, diagrams and 3D.\n\n\
             To change an earlier animation, pass its `name` with the whole new source. Rendering \
             takes a minute or more.{note}"
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "engine": { "type": "string", "enum": ["html", "manim"] },
                "source": { "type": "string", "description": "The whole composition (HTML) or scene (Python)." },
                "name": { "type": "string", "description": "A short name, letters, digits and dashes; reuse it to change an animation." },
                "format": { "type": "string", "enum": ["mp4", "gif"], "default": "mp4" }
            },
            "required": ["engine", "source"]
        }),
        connector_id: None,
        tool_name: MAKE_ANIMATION.into(),
        read_only: false,
    }
}

/// A safe folder name from the model's.
pub(in crate::chat::multimedia) fn slug(name: &str) -> String {
    let s = crate::support::names::slug(name, 48);
    if s.is_empty() {
        format!("animation-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))
    } else {
        s
    }
}

/// Checks an HTML composition for what HyperFrames needs, so a mistake is told back to the model
/// before minutes of rendering.
pub(in crate::chat::multimedia) fn check_html(source: &str) -> Result<(), ChatError> {
    let lower = source.to_lowercase();
    if !lower.contains("data-composition-id") {
        return Err(ChatError::Invalid(
            "The root element needs data-composition-id=\"main\".".into(),
        ));
    }
    if !lower.contains("data-duration") {
        return Err(ChatError::Invalid(
            "The root element needs data-duration (seconds).".into(),
        ));
    }
    if !source.contains("__timelines") {
        return Err(ChatError::Invalid(
            "Register the timeline: window.__timelines[\"main\"] = tl;".into(),
        ));
    }
    Ok(())
}

/// The command that renders `dir` into `out`.
fn command(
    engine: &str,
    dir: &Path,
    out: &Path,
    format: &str,
) -> Result<tokio::process::Command, ChatError> {
    match engine {
        "html" => {
            let npx = locate("npx").ok_or_else(|| ChatError::Invalid("HyperFrames needs Node.js 22 or later (it brings npx): install it from nodejs.org, then ask again.".into()))?;
            let mut c = tokio::process::Command::new(npx);
            c.args(["--yes", HYPERFRAMES, "render", "."]).arg("-o").arg(out).args(["--format", format, "--quality", "looks", "--quiet"]);
            Ok(c)
        }
        "manim" => {
            let bin = locate("manimgx").ok_or_else(|| ChatError::Invalid("ManimGX is not installed: run `uv tool install manimgx` (Python 3.13+), then ask again.".into()))?;
            let mut c = tokio::process::Command::new(bin);
            c.args(["render", "scene.py", "Main"]);
            let _ = (out, format);
            Ok(c)
        }
        other => Err(ChatError::Invalid(format!("Unknown engine `{other}`: use html or manim."))),
    }
    .map(|mut c| {
        c.current_dir(dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        c
    })
}

pub async fn make(config_dir: &Path, conversation_id: &str, args: &Value) -> Outcome {
    match run(config_dir, conversation_id, args).await {
        Ok(text) => Outcome {
            text,
            is_error: false,
        },
        Err(e) => Outcome::err(e),
    }
}

async fn run(config_dir: &Path, conversation_id: &str, args: &Value) -> Result<String, ChatError> {
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let engine = s("engine");
    let source = args
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if source.trim().is_empty() {
        return Err(ChatError::Invalid(
            "Give the whole composition or scene as `source`.".into(),
        ));
    }
    let format = if s("format") == "gif" && engine == "html" {
        "gif"
    } else {
        "mp4"
    };
    let name = slug(&s("name"));
    let workspace =
        crate::chat::sandbox::workspace(config_dir, conversation_id).map_err(ChatError::Invalid)?;
    let dir = workspace.join("animations").join(&name);
    std::fs::create_dir_all(&dir).map_err(ChatError::internal)?;
    match engine.as_str() {
        "html" => {
            check_html(&source)?;
            std::fs::write(dir.join("index.html"), &source).map_err(ChatError::internal)?;
            // A project file, so the folder also opens in HyperFrames' own tools.
            if !dir.join("hyperframes.json").exists() {
                let _ = std::fs::write(
                    dir.join("hyperframes.json"),
                    json!({ "name": name }).to_string(),
                );
            }
        }
        "manim" => {
            if !source.contains("class Main") {
                return Err(ChatError::Invalid(
                    "Name the scene class Main (class Main(m.Scene)).".into(),
                ));
            }
            std::fs::write(dir.join("scene.py"), &source).map_err(ChatError::internal)?;
        }
        other => {
            return Err(ChatError::Invalid(format!(
                "Unknown engine `{other}`: use html or manim."
            )))
        }
    }
    let videos = workspace.join("videos");
    std::fs::create_dir_all(&videos).map_err(ChatError::internal)?;
    let file = format!("{name}-{}.{format}", chrono::Local::now().format("%H%M%S"));
    let target = videos.join(&file);
    let started = std::time::Instant::now();
    let output = tokio::time::timeout(
        RENDER_LIMIT,
        command(&engine, &dir, &target, format)?.output(),
    )
    .await
    .map_err(|_| {
        "The render took longer than 20 minutes and was stopped; make it shorter or simpler."
            .to_string()
    })
    .map_err(ChatError::Invalid)?
    .map_err(|e| ChatError::Invalid(format!("The renderer could not start: {e}")))?;
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if engine == "manim" {
        // ManimGX writes <Scene>.mp4 beside the scene.
        let made = dir.join("Main.mp4");
        if made.exists() {
            std::fs::rename(&made, &target)
                .or_else(|_| std::fs::copy(&made, &target).map(|_| ()))
                .map_err(ChatError::internal)?;
        }
    }
    if !output.status.success() || !target.exists() {
        let tail: Vec<&str> = log
            .lines()
            .filter(|l| !l.starts_with("@hf-") && !l.trim().is_empty())
            .collect();
        let tail = tail[tail.len().saturating_sub(15)..].join("\n");
        return Err(ChatError::Invalid(format!(
            "The render failed. Fix the source and try again.\n{tail}"
        )));
    }
    Ok(json!({
        "file": format!("videos/{file}"),
        "source": format!("animations/{name}/{}", if engine == "html" { "index.html" } else { "scene.py" }),
        "name": name,
        "engine": engine,
        "seconds_to_render": started.elapsed().as_secs(),
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_checks() {
        assert_eq!(slug("Q3 Sales — chart!"), "q3-sales-chart");
        assert!(slug("  ").starts_with("animation-"));
        assert!(check_html("<div data-composition-id=\"main\" data-duration=\"5\"></div><script>window.__timelines['main']=tl</script>").is_ok());
        assert!(check_html("<div data-duration=\"5\"></div>")
            .unwrap_err()
            .to_string()
            .contains("data-composition-id"));
        assert!(
            check_html("<div data-composition-id=\"main\" data-duration=\"5\"></div>")
                .unwrap_err()
                .to_string()
                .contains("__timelines")
        );
    }

    /// With Node.js and network (`CORTEX_TEST_ANIMATION=1`): a real HyperFrames render.
    #[tokio::test]
    #[ignore]
    async fn hyperframes_renders_a_composition() {
        if std::env::var("CORTEX_TEST_ANIMATION").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let html = r##"<!doctype html><html><head><script src="https://cdn.jsdelivr.net/npm/gsap@3.15.0/dist/gsap.min.js"></script>
<style>html,body{margin:0;width:640px;height:360px;background:#123}#t{color:#fff;font:600 48px sans-serif;position:absolute;top:150px;left:60px}</style></head>
<body><div id="root" data-composition-id="main" data-start="0" data-duration="2" data-width="640" data-height="360">
<h1 id="t" class="clip" data-start="0" data-duration="2" data-track-index="0">Cortex</h1></div>
<script>const tl=gsap.timeline({paused:true});tl.fromTo("#t",{opacity:0},{opacity:1,duration:1},0);window.__timelines["main"]=tl;tl.seek(0);</script></body></html>"##;
        let out = make(
            dir.path(),
            "00000000-0000-4000-8000-000000000002",
            &json!({ "engine": "html", "source": html, "name": "test" }),
        )
        .await;
        assert!(!out.is_error, "{}", out.text);
        let v: Value = serde_json::from_str(&out.text).unwrap();
        let ws =
            crate::chat::sandbox::workspace(dir.path(), "00000000-0000-4000-8000-000000000002")
                .unwrap();
        assert!(
            std::fs::metadata(ws.join(v["file"].as_str().unwrap()))
                .unwrap()
                .len()
                > 1000
        );
    }
}
