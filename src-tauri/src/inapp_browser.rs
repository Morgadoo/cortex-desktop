//! Cortex's own browser window (Vibe's integrated browser): links from answers open beside the
//! chat instead of in another app, with Back, Forward, Reload, Copy link and "Open in external
//! browser" in the window's menu. Off unless chosen (Settings → General); otherwise links ask
//! before opening in the system browser.
//!
//! The pages are remote sites: no capability names a remote origin, so they get no access to
//! the app's commands.

use std::path::PathBuf;

use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "cortex-browser";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub in_app: bool,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    crate::settings::get_config_dir(app)
        .ok()
        .map(|d| d.join("chat").join("browser.json"))
}

pub fn prefs(app: &AppHandle) -> Prefs {
    path(app)
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, prefs: &Prefs) -> Result<(), String> {
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

/// Only web pages open here; anything else (mailto:, files) goes to the system.
pub fn is_web(url: &url::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Opens `url` in the browser window, reusing it when it is open.
pub fn open(app: &AppHandle, url: &url::Url) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.navigate(url.clone()).map_err(|e| e.to_string())?;
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }
    let menu = {
        let back = MenuItemBuilder::with_id("back", "Back")
            .accelerator("Alt+Left")
            .build(app)
            .map_err(|e| e.to_string())?;
        let forward = MenuItemBuilder::with_id("forward", "Forward")
            .accelerator("Alt+Right")
            .build(app)
            .map_err(|e| e.to_string())?;
        let reload = MenuItemBuilder::with_id("reload", "Reload")
            .accelerator("CmdOrCtrl+R")
            .build(app)
            .map_err(|e| e.to_string())?;
        let copy = MenuItemBuilder::with_id("copy-link", "Copy link")
            .build(app)
            .map_err(|e| e.to_string())?;
        let external = MenuItemBuilder::with_id("external", "Open in external browser")
            .accelerator("CmdOrCtrl+Shift+O")
            .build(app)
            .map_err(|e| e.to_string())?;
        let close = MenuItemBuilder::with_id("close", "Close")
            .accelerator("CmdOrCtrl+W")
            .build(app)
            .map_err(|e| e.to_string())?;
        let page = SubmenuBuilder::new(app, "Page")
            .items(&[&back, &forward, &reload])
            .separator()
            .items(&[&copy, &external])
            .separator()
            .item(&close)
            .build()
            .map_err(|e| e.to_string())?;
        MenuBuilder::new(app)
            .item(&page)
            .build()
            .map_err(|e| e.to_string())?
    };
    let handle = app.clone();
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url.clone()))
        .title(url.host_str().unwrap_or("Browser"))
        .inner_size(1100.0, 820.0)
        .menu(menu)
        // Links that would open yet another window load here instead.
        .on_new_window(move |url, _| {
            if let Some(w) = handle.get_webview_window(LABEL) {
                let _ = w.navigate(url);
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_page_load(|window, payload| {
            if let Some(host) = payload.url().host_str() {
                let _ = window.set_title(host);
            }
        })
        .build()
        .map_err(|e| e.to_string())?;
    window.on_menu_event(|window, event| {
        let Some(window) = window.app_handle().get_webview_window(LABEL) else {
            return;
        };
        let current = window.url().ok();
        match event.id().as_ref() {
            "back" => {
                let _ = window.eval("history.back()");
            }
            "forward" => {
                let _ = window.eval("history.forward()");
            }
            "reload" => {
                let _ = window.eval("location.reload()");
            }
            "copy-link" => {
                if let Some(url) = current {
                    let _ = copy_text(url.as_str());
                }
            }
            "external" => {
                if let Some(url) = current {
                    if let Err(e) = open::that_detached(url.as_str()) {
                        warn!(
                            "could not open the page in the system browser: {:?}",
                            e.kind()
                        );
                    }
                }
            }
            "close" => {
                let _ = window.close();
            }
            _ => {}
        }
    });
    info!(
        "opened Cortex's browser on {}",
        url.host_str().unwrap_or_default()
    );
    Ok(())
}

/// Puts text on the clipboard through the system's tools; false when none is installed.
pub(crate) fn copy_text(text: &str) -> bool {
    use std::io::Write as _;
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };
    for (program, args) in tools {
        if let Ok(mut child) = std::process::Command::new(program)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().is_ok_and(|s| s.success());
        }
    }
    warn!("no clipboard tool was found to copy the text");
    false
}

#[tauri::command]
pub fn browser_prefs(app: AppHandle) -> Prefs {
    prefs(&app)
}

#[tauri::command]
pub fn set_browser_prefs(app: AppHandle, prefs: Prefs) -> Result<(), String> {
    save(&app, &prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_open_inside() {
        assert!(is_web(&"https://example.com/a".parse().unwrap()));
        assert!(is_web(&"http://example.com".parse().unwrap()));
        assert!(!is_web(&"mailto:a@example.com".parse().unwrap()));
        assert!(!is_web(&"file:///etc/passwd".parse().unwrap()));
    }
}
