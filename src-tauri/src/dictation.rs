//! System-wide dictation (Osaurus's dictation hotkey): a global shortcut, pressed in any app,
//! opens a small "Listening" pill that does not take focus. Pressing the shortcut again (or
//! pausing, if chosen) ends it; with hold-to-talk, letting go of the shortcut ends it, and a
//! quick tap still toggles. The speech is transcribed with the speech-to-text set in
//! Settings → Audio and typed into the app that had focus, pasted, or left on the clipboard.
//!
//! Off until chosen (Settings → Audio → Dictation anywhere). Typing uses the system's tools:
//! xdotool on X11, wtype or ydotool on Wayland; on macOS and Windows the text is pasted. GNOME
//! on Wayland offers no virtual keyboard to wtype, and ydotool types by key code, which garbles
//! accents and other layouts, so there the text is pasted by default.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder, Window,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub const LABEL: &str = "dictation";

/// The pill window, in logical pixels (the pill plus room for its shadow).
const WIDTH: f64 = 440.0;
const HEIGHT: f64 = 84.0;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Insert {
    /// Typed key by key into the app that had focus.
    #[default]
    Type,
    /// Put on the clipboard and pasted with the paste shortcut.
    Paste,
    /// Left on the clipboard only.
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub enabled: bool,
    pub shortcut: String,
    pub insert: Insert,
    /// Finish after a pause in speech, as well as on the second press.
    pub stop_on_pause: bool,
    /// Listen while the shortcut is held down and finish when it is let go.
    pub hold_to_talk: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            enabled: false,
            shortcut: "Ctrl+Alt+D".into(),
            insert: default_insert(
                &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
                &std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
            ),
            stop_on_pause: false,
            hold_to_talk: false,
        }
    }
}

/// How dictated text goes in when the user has not chosen: pasted on GNOME under Wayland (see
/// the module notes), typed elsewhere.
pub fn default_insert(desktop: &str, session_type: &str) -> Insert {
    let gnome = desktop.split(':').any(|d| d.eq_ignore_ascii_case("gnome"));
    if gnome && session_type.eq_ignore_ascii_case("wayland") {
        Insert::Paste
    } else {
        Insert::Type
    }
}

/// A press shorter than this, with hold-to-talk on, is a tap: it toggles as without it, so
/// the same shortcut can still start a long dictation that ends on a pause or a second tap.
const TAP: Duration = Duration::from_millis(350);

/// Whether letting go of the shortcut, held for `held`, finishes listening.
pub fn release_finishes(hold_to_talk: bool, held: Duration) -> bool {
    hold_to_talk && held >= TAP
}

/// What the app is doing with the shortcut right now.
#[derive(Default)]
pub struct Dictation {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    shortcut: Option<Shortcut>,
    listening: bool,
    hold_to_talk: bool,
    /// When the shortcut went down, while it is held.
    pressed_at: Option<Instant>,
    /// The X11 window that had focus when listening began, to type into.
    target: Option<String>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    crate::settings::get_config_dir(app)
        .ok()
        .map(|d| d.join("chat").join("dictation.json"))
}

pub fn prefs(app: &AppHandle) -> Prefs {
    path(app)
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, prefs: &Prefs) -> Result<(), String> {
    let path = path(app).ok_or("no config folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Registers the saved shortcut at startup, when dictation is on.
pub fn startup(app: &AppHandle) {
    let p = prefs(app);
    if !p.enabled {
        return;
    }
    match p.shortcut.parse::<Shortcut>() {
        Ok(shortcut) => match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                if let Some(state) = app.try_state::<Dictation>() {
                    let mut inner = state.inner.lock().unwrap();
                    inner.shortcut = Some(shortcut);
                    inner.hold_to_talk = p.hold_to_talk;
                }
            }
            Err(e) => warn!("could not register the dictation shortcut: {e}"),
        },
        Err(e) => warn!("the dictation shortcut is not valid: {e}"),
    }
}

/// Whether `shortcut` is the dictation one (the plugin has one handler for every shortcut).
pub fn is_shortcut(app: &AppHandle, shortcut: &Shortcut) -> bool {
    app.try_state::<Dictation>()
        .is_some_and(|s| s.inner.lock().unwrap().shortcut.as_ref() == Some(shortcut))
}

/// The shortcut went down: start listening, or finish.
pub fn pressed(app: &AppHandle) {
    if let Some(state) = app.try_state::<Dictation>() {
        state.inner.lock().unwrap().pressed_at = Some(Instant::now());
    }
    toggle(app);
}

/// The shortcut came up: with hold-to-talk, finish unless it was only a tap.
pub fn released(app: &AppHandle) {
    let Some(state) = app.try_state::<Dictation>() else {
        return;
    };
    let finish = {
        let mut inner = state.inner.lock().unwrap();
        let held = inner.pressed_at.take().map(|t| t.elapsed());
        inner.listening && held.is_some_and(|h| release_finishes(inner.hold_to_talk, h))
    };
    if finish {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.emit("dictation://stop", ());
        }
    }
}

/// Start listening, or finish.
fn toggle(app: &AppHandle) {
    let Some(state) = app.try_state::<Dictation>() else {
        return;
    };
    let listening = state.inner.lock().unwrap().listening;
    if listening {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.emit("dictation://stop", ());
        }
        return;
    }
    let target = focused_window();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.listening = true;
        inner.target = target;
    }
    if let Err(e) = show(app) {
        warn!("could not open the dictation pill: {e}");
        state.inner.lock().unwrap().listening = false;
    }
}

fn show(app: &AppHandle) -> tauri::Result<()> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => {
            window.show()?;
            window.emit("dictation://start", ())?;
            window
        }
        // A new window starts listening as soon as its page loads.
        None => {
            let window =
                WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("dictate.html".into()))
                    .title("Dictation")
                    .decorations(false)
                    .skip_taskbar(true)
                    .always_on_top(true)
                    .transparent(true)
                    .shadow(false)
                    .resizable(false)
                    .focused(false)
                    .inner_size(WIDTH, HEIGHT)
                    .build()?;
            #[cfg(target_os = "linux")]
            crate::main_window::allow_microphone(&window);
            #[cfg(target_os = "macos")]
            crate::microphone::install(&window);
            window
        }
    };
    let monitor = window
        .cursor_position()
        .ok()
        .and_then(|c| window.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        // From the known size: a window not yet mapped can report none.
        let scale = monitor.scale_factor();
        let (w, h) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
        let x = monitor.position().x + (monitor.size().width as i32 - w) / 2;
        let y = monitor.position().y + monitor.size().height as i32
            - h
            - ((if cfg!(target_os = "macos") {
                150.0
            } else {
                60.0
            }) * scale) as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

/// The X11 window with focus, to return to before typing; `None` elsewhere.
fn focused_window() -> Option<String> {
    if !cfg!(target_os = "linux") || wayland() {
        return None;
    }
    // The window manager's idea of the active window, else the X input focus (no window manager).
    ["getactivewindow", "getwindowfocus"]
        .iter()
        .find_map(|verb| {
            let out = Command::new("xdotool")
                .arg(verb)
                .stderr(Stdio::null())
                .output()
                .ok()?;
            let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (out.status.success() && !id.is_empty()).then_some(id)
        })
}

fn wayland() -> bool {
    is_wayland(
        &std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}

/// Whether the desktop session is Wayland. `DISPLAY` says nothing: Xwayland sets it in every
/// GNOME and KDE Wayland session, and xdotool there reaches only X11 windows.
pub fn is_wayland(session_type: &str, wayland_display: bool) -> bool {
    match session_type.to_ascii_lowercase().as_str() {
        "wayland" => true,
        "x11" => false,
        _ => wayland_display,
    }
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed"))
    }
}

/// The commands that type `text`, in the order to try them; empty where typing is not offered.
pub fn type_commands(text: &str, os: &str, wayland: bool) -> Vec<Vec<String>> {
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match os {
        "linux" if wayland => vec![
            v(&["wtype", "--", text]),
            v(&["ydotool", "type", "--", text]),
        ],
        "linux" => vec![v(&[
            "xdotool",
            "type",
            "--clearmodifiers",
            "--delay",
            "6",
            "--",
            text,
        ])],
        _ => Vec::new(),
    }
}

/// The commands that press the paste shortcut, in the order to try them.
pub fn paste_commands(os: &str, wayland: bool) -> Vec<Vec<String>> {
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match os {
        "macos" => vec![v(&["osascript", "-e", "tell application \"System Events\" to keystroke \"v\" using command down"])],
        "windows" => vec![v(&["powershell", "-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^v')"])],
        _ if wayland => vec![v(&["wtype", "-M", "ctrl", "v", "-m", "ctrl"]), v(&["ydotool", "key", "29:1", "47:1", "47:0", "29:0"])],
        _ => vec![v(&["xdotool", "key", "--clearmodifiers", "ctrl+v"])],
    }
}

fn first_that_works(commands: &[Vec<String>]) -> bool {
    commands.iter().any(|c| {
        run(
            &c[0],
            &c[1..].iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .is_ok()
    })
}

/// Puts the text where the user was; returns how it went, for the pill to show.
fn insert(text: &str, how: Insert, target: Option<&str>) -> &'static str {
    if let Some(id) = target {
        if run("xdotool", &["windowactivate", "--sync", id]).is_err() {
            let _ = run("xdotool", &["windowfocus", "--sync", id]);
        }
    }
    // Let the shortcut's keys come up and focus settle before keys are sent.
    std::thread::sleep(std::time::Duration::from_millis(250));
    let os = std::env::consts::OS;
    if how == Insert::Type && first_that_works(&type_commands(text, os, wayland())) {
        return "typed";
    }
    if !crate::inapp_browser::copy_text(text) {
        return "failed";
    }
    if how != Insert::Copy && first_that_works(&paste_commands(os, wayland())) {
        return "pasted";
    }
    "copied"
}

fn end(state: &Dictation) -> Option<String> {
    let mut inner = state.inner.lock().unwrap();
    inner.listening = false;
    inner.target.take()
}

#[tauri::command]
pub fn dictation_prefs(app: AppHandle) -> Prefs {
    prefs(&app)
}

#[tauri::command]
pub fn set_dictation_prefs(
    app: AppHandle,
    state: State<'_, Dictation>,
    prefs: Prefs,
) -> Result<(), String> {
    let new = if prefs.enabled {
        Some(
            prefs
                .shortcut
                .parse::<Shortcut>()
                .map_err(|e| format!("That shortcut is not valid: {e}"))?,
        )
    } else {
        None
    };
    let old = state.inner.lock().unwrap().shortcut;
    if old != new {
        if let Some(old) = old {
            let _ = app.global_shortcut().unregister(old);
            #[cfg(target_os = "linux")]
            let _ = app.global_shortcut().unregister(old);
        }
        if let Some(new) = new {
            if let Err(e) = app.global_shortcut().register(new) {
                if let Some(old) = old {
                    let _ = app.global_shortcut().register(old);
                }
                return Err(format!(
                    "That shortcut could not be registered (another app may use it): {e}"
                ));
            }
        }
        state.inner.lock().unwrap().shortcut = new;
    }
    state.inner.lock().unwrap().hold_to_talk = prefs.hold_to_talk;
    save(&app, &prefs)
}

/// From the pill: the transcript, to put where the user was.
// IPC: own permission (see permissions/windows.toml)
#[tauri::command]
pub async fn dictation_insert(
    app: AppHandle,
    window: Window,
    text: String,
) -> Result<String, String> {
    if window.label() != LABEL {
        return Err("Only the dictation pill can insert text.".into());
    }
    let state = app.state::<Dictation>();
    let target = end(&state);
    let _ = window.hide();
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok("empty".into());
    }
    let how = prefs(&app).insert;
    let done = tauri::async_runtime::spawn_blocking(move || insert(&text, how, target.as_deref()))
        .await
        .map_err(|e| e.to_string())?;
    info!("dictation: {done}");
    if done == "copied" || done == "failed" {
        let body = if done == "copied" {
            "The dictated text is on the clipboard."
        } else {
            "The dictated text could not be typed or copied."
        };
        crate::chat::engine::notify(&app, "Dictation", body);
    }
    Ok(done.into())
}

/// From the pill: listening ended without text (cancelled, or an error shown).
// IPC: own permission (see permissions/windows.toml)
#[tauri::command]
pub fn dictation_cancel(app: AppHandle, window: Window) {
    if window.label() == LABEL {
        end(&app.state::<Dictation>());
        let _ = window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_pasting_use_each_systems_tools() {
        assert_eq!(
            type_commands("héllo", "linux", false),
            vec![vec![
                "xdotool",
                "type",
                "--clearmodifiers",
                "--delay",
                "6",
                "--",
                "héllo"
            ]]
        );
        assert_eq!(
            type_commands("-x", "linux", true)[0],
            vec!["wtype", "--", "-x"]
        );
        assert!(
            type_commands("x", "macos", false).is_empty()
                && type_commands("x", "windows", false).is_empty()
        );
        assert_eq!(
            paste_commands("linux", false),
            vec![vec!["xdotool", "key", "--clearmodifiers", "ctrl+v"]]
        );
        assert!(paste_commands("macos", false)[0][2].contains("command down"));
        assert!(paste_commands("windows", false)[0][3].contains("^v"));
        assert_eq!(
            paste_commands("linux", true)[1],
            vec!["ydotool", "key", "29:1", "47:1", "47:0", "29:0"]
        );
    }

    #[test]
    fn prefs_default_off_and_read_old_files() {
        let p: Prefs = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(
            p.enabled
                && p.shortcut == "Ctrl+Alt+D"
                && p.insert == Prefs::default().insert
                && !p.stop_on_pause
                && !p.hold_to_talk
        );
        assert!(!Prefs::default().enabled);
        assert!(Prefs::default().shortcut.parse::<Shortcut>().is_ok());
        let p: Prefs = serde_json::from_str(r#"{"insert":"paste","stopOnPause":true}"#).unwrap();
        assert_eq!((p.insert, p.stop_on_pause), (Insert::Paste, true));
        let p: Prefs = serde_json::from_str(r#"{"holdToTalk":true}"#).unwrap();
        assert!(p.hold_to_talk);
    }

    #[test]
    fn gnome_on_wayland_pastes_and_everywhere_else_types() {
        assert_eq!(default_insert("GNOME", "wayland"), Insert::Paste);
        assert_eq!(default_insert("ubuntu:GNOME", "wayland"), Insert::Paste);
        assert_eq!(default_insert("GNOME", "x11"), Insert::Type);
        assert_eq!(default_insert("KDE", "wayland"), Insert::Type);
        assert_eq!(default_insert("", ""), Insert::Type);
        // A chosen way is kept wherever the app runs.
        let p: Prefs = serde_json::from_str(r#"{"insert":"type"}"#).unwrap();
        assert_eq!(p.insert, Insert::Type);
    }

    #[test]
    fn a_wayland_session_is_wayland_even_with_xwayland_s_display() {
        assert!(is_wayland("wayland", false));
        assert!(!is_wayland("x11", true));
        assert!(is_wayland("", true) && !is_wayland("", false));
        assert!(is_wayland("Wayland", true));
    }

    #[test]
    fn hold_to_talk_finishes_on_release_but_a_tap_still_toggles() {
        assert!(release_finishes(true, Duration::from_millis(900)));
        assert!(!release_finishes(true, Duration::from_millis(120)));
        assert!(!release_finishes(false, Duration::from_secs(3)));
    }
}
