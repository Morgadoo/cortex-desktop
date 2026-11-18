//! Computer use: the assistant sees the screen and uses the mouse and keyboard, as OpenBot and
//! Anthropic's computer-use tool allow.
//!
//! Off by default (Settings → Permissions). Screenshots are reads; every other action goes
//! through the same approval gate as any connector tool, so in Ask mode each click asks, and in
//! Auto each one is checked against the request. After an action the screen is captured again,
//! so the model sees what happened. Screenshots go to the model scaled to at most
//! [`MAX_WIDTH`] pixels wide as JPEG; coordinates it sends back are in that scaled space.
//!
//! Uses the platform's own tools: on Linux `xdotool` with ImageMagick `import` (X11) or `grim`
//! (Wayland, screenshots only); on macOS `screencapture` and `cliclick`. When one is missing the
//! tool says what to install.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;

use super::store::{chat_dir, write_json};
use super::tools::ChatTool;

pub const CONNECTOR: &str = "computer";
pub const TOOL: &str = "computer";
/// Screenshots wider than this are scaled down for the model.
pub const MAX_WIDTH: u32 = 1280;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ComputerSettings {
    pub enabled: bool,
}

fn settings_path(config_dir: &Path) -> std::path::PathBuf {
    chat_dir(config_dir).join("computer.json")
}

pub fn load(config_dir: &Path) -> ComputerSettings {
    std::fs::read_to_string(settings_path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &ComputerSettings) -> Result<(), String> {
    write_json(&settings_path(config_dir), settings)
}

pub fn definition() -> ChatTool {
    ChatTool {
        name: TOOL.into(),
        description: "See and use this computer's screen, mouse and keyboard. Start with a screenshot; \
            coordinates are pixels in the latest screenshot. After each action a new screenshot \
            comes back. Prefer keyboard shortcuts when they are reliable. Use it only for what \
            the user asked; other tools are better for files, commands and the web."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["screenshot", "click", "double_click", "right_click", "move", "type", "key", "scroll"] },
                "x": { "type": "integer", "description": "For click, double_click, right_click, move and scroll." },
                "y": { "type": "integer" },
                "text": { "type": "string", "description": "For type: the text to type." },
                "keys": { "type": "string", "description": "For key: a combination like ctrl+s, Return or alt+Tab." },
                "direction": { "type": "string", "enum": ["up", "down"], "description": "For scroll." },
                "amount": { "type": "integer", "minimum": 1, "maximum": 20, "description": "For scroll: how many steps." }
            },
            "required": ["action"]
        }),
        connector_id: Some(CONNECTOR.into()),
        tool_name: TOOL.into(),
        read_only: false,
    }
}

/// Whether this call only looks.
pub fn is_screenshot(input: &Value) -> bool {
    input.get("action").and_then(Value::as_str) == Some("screenshot")
}

/// Per conversation: the latest screenshot (JPEG, base64) waiting to go to the model, and the
/// factor from screenshot pixels to screen pixels.
fn shots() -> &'static Mutex<HashMap<String, String>> {
    static SHOTS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    SHOTS.get_or_init(Default::default)
}

fn scales() -> &'static Mutex<HashMap<String, f64>> {
    static SCALES: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
    SCALES.get_or_init(Default::default)
}

/// The screenshot the last call took, for the tool result; taken once.
pub fn take_image(conversation_id: &str) -> Option<String> {
    shots().lock().ok()?.remove(conversation_id)
}

async fn output(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new(program).args(args).output().await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("`{program}` is not installed. {}", install_hint()),
        _ => format!("{program}: {e}"),
    })?;
    if !out.status.success() {
        return Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Computer use on macOS needs cliclick (brew install cliclick) and screen-recording and accessibility permission for this app."
    } else if cfg!(target_os = "linux") {
        "Computer use on Linux needs xdotool and ImageMagick (X11), or grim for screenshots on Wayland."
    } else {
        "Computer use is available on Linux and macOS."
    }
}

/// The screen as PNG bytes.
async fn capture() -> Result<Vec<u8>, String> {
    if cfg!(target_os = "macos") {
        let path = std::env::temp_dir().join(format!("lumo-screen-{}.png", uuid::Uuid::new_v4().simple()));
        let p = path.to_string_lossy().into_owned();
        output("screencapture", &["-x", "-t", "png", &p]).await?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&path);
        return bytes;
    }
    if cfg!(target_os = "linux") {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return output("grim", &["-t", "png", "-"]).await;
        }
        return output("import", &["-window", "root", "png:-"]).await;
    }
    Err(install_hint().into())
}

/// Scales a PNG to at most `max_width` wide as JPEG; returns base64 and the scale back to screen.
pub fn shrink(png: &[u8], max_width: u32) -> Result<(String, f64, u32, u32), String> {
    let image = image::load_from_memory(png).map_err(|e| format!("The screenshot could not be read: {e}"))?;
    let (w, h) = (image.width(), image.height());
    let scale = if w > max_width { f64::from(w) / f64::from(max_width) } else { 1.0 };
    let small = if scale > 1.0 { image.resize(max_width, (f64::from(h) / scale).round() as u32, image::imageops::FilterType::Triangle) } else { image };
    let mut jpeg = Vec::new();
    small.to_rgb8().write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    Ok((base64::engine::general_purpose::STANDARD.encode(jpeg), scale, small.width(), small.height()))
}

async fn screenshot(conversation_id: &str) -> Result<String, String> {
    let png = capture().await?;
    let (data, scale, w, h) = shrink(&png, MAX_WIDTH)?;
    if let Ok(mut s) = shots().lock() {
        s.insert(conversation_id.to_string(), data);
    }
    if let Ok(mut s) = scales().lock() {
        s.insert(conversation_id.to_string(), scale);
    }
    Ok(format!("Screenshot attached ({w}×{h}). Coordinates are in this image's pixels."))
}

/// Screen coordinates for a point in the latest screenshot.
fn point(conversation_id: &str, input: &Value) -> Result<(i64, i64), String> {
    let x = input.get("x").and_then(Value::as_i64).ok_or("Give x and y from the latest screenshot.")?;
    let y = input.get("y").and_then(Value::as_i64).ok_or("Give x and y from the latest screenshot.")?;
    let scale = scales().lock().ok().and_then(|s| s.get(conversation_id).copied()).unwrap_or(1.0);
    Ok(((x as f64 * scale).round() as i64, (y as f64 * scale).round() as i64))
}

async fn act(conversation_id: &str, action: &str, input: &Value) -> Result<String, String> {
    let mac = cfg!(target_os = "macos");
    let text = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match action {
        "click" | "double_click" | "right_click" | "move" => {
            let (x, y) = point(conversation_id, input)?;
            if mac {
                let verb = match action { "click" => "c", "double_click" => "dc", "right_click" => "rc", _ => "m" };
                output("cliclick", &[&format!("{verb}:{x},{y}")]).await?;
            } else {
                let (xs, ys) = (x.to_string(), y.to_string());
                let mut args = vec!["mousemove", "--sync", xs.as_str(), ys.as_str()];
                match action {
                    "click" => args.extend(["click", "1"]),
                    "double_click" => args.extend(["click", "--repeat", "2", "1"]),
                    "right_click" => args.extend(["click", "3"]),
                    _ => {}
                }
                output("xdotool", &args).await?;
            }
            Ok(format!("{action} at {x},{y} (screen pixels)."))
        }
        "type" => {
            let t = text("text");
            if t.is_empty() {
                return Err("Give the text to type.".into());
            }
            if mac {
                output("cliclick", &[&format!("t:{t}")]).await?;
            } else {
                output("xdotool", &["type", "--delay", "12", "--", &t]).await?;
            }
            Ok(format!("Typed {} characters.", t.chars().count()))
        }
        "key" => {
            let keys = text("keys");
            if keys.is_empty() {
                return Err("Give the keys, like ctrl+s.".into());
            }
            if mac {
                output("cliclick", &[&format!("kp:{}", keys.to_lowercase())]).await?;
            } else {
                output("xdotool", &["key", "--", &keys]).await?;
            }
            Ok(format!("Pressed {keys}."))
        }
        "scroll" => {
            let amount = input.get("amount").and_then(Value::as_u64).unwrap_or(3).clamp(1, 20).to_string();
            let down = text("direction") != "up";
            if input.get("x").is_some() {
                let (x, y) = point(conversation_id, input)?;
                if !mac {
                    output("xdotool", &["mousemove", "--sync", &x.to_string(), &y.to_string()]).await?;
                }
            }
            if mac {
                return Err("Scrolling is not available on macOS yet; use the arrow or Page keys.".into());
            }
            output("xdotool", &["click", "--repeat", &amount, if down { "5" } else { "4" }]).await?;
            Ok(format!("Scrolled {} {amount} steps.", if down { "down" } else { "up" }))
        }
        other => Err(format!("Unknown action {other}.")),
    }
}

/// Runs one computer action; every action but a screenshot is followed by a fresh screenshot.
pub async fn run(conversation_id: &str, input: &Value) -> Result<String, String> {
    let action = input.get("action").and_then(Value::as_str).unwrap_or("screenshot");
    if action == "screenshot" {
        return screenshot(conversation_id).await;
    }
    let done = act(conversation_id, action, input).await?;
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let shot = screenshot(conversation_id).await.unwrap_or_else(|e| format!("(No new screenshot: {e})"));
    Ok(format!("{done} {shot}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_shrink_and_map_back() {
        let big = image::RgbImage::from_pixel(2560, 1440, image::Rgb([30, 30, 30]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(big).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (data, scale, w, h) = shrink(&png, MAX_WIDTH).unwrap();
        assert_eq!((w, h), (1280, 720));
        assert!((scale - 2.0).abs() < 1e-9);
        assert!(!data.is_empty());
        scales().lock().unwrap().insert("c".into(), scale);
        assert_eq!(point("c", &json!({ "x": 100, "y": 50 })).unwrap(), (200, 100));
        assert!(point("c", &json!({ "x": 1 })).is_err());
        assert!(is_screenshot(&json!({ "action": "screenshot" })) && !is_screenshot(&json!({ "action": "click" })));
    }
}
