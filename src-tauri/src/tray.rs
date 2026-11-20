//! The app menu, the tray icon and its menu.

use log::{error, warn};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Theme};

use crate::dialog::branded;
use crate::{main_window, settings, updater};

const TRAY_ICON_ID: &str = "cortex-tray";
const TRAY_ICON_LIGHT: Image<'static> = tauri::include_image!("icons/tray-icon-light.png");
const TRAY_ICON_DARK: Image<'static> = tauri::include_image!("icons/tray-icon-dark.png");
// The macOS menu bar follows the wallpaper, not the app theme, so let AppKit tint the icon.
const TRAY_ICON_IS_TEMPLATE: bool = cfg!(target_os = "macos");

fn icon_for_theme(theme: Theme) -> Image<'static> {
    match theme {
        Theme::Dark if !TRAY_ICON_IS_TEMPLATE => TRAY_ICON_DARK,
        _ => TRAY_ICON_LIGHT,
    }
}

fn system_theme(app: &AppHandle) -> Theme {
    app.get_webview_window(main_window::LABEL)
        .and_then(|window| window.theme().ok())
        .unwrap_or(Theme::Light)
}

pub fn sync_icon_for_theme(app: &AppHandle) {
    if TRAY_ICON_IS_TEMPLATE {
        return;
    }
    let Some(tray) = app.tray_by_id(TRAY_ICON_ID) else {
        return;
    };
    if let Err(e) = tray.set_icon(Some(icon_for_theme(system_theme(app)))) {
        warn!("failed to update tray icon for theme: {e}");
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Show App", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, Some("CmdOrCtrl+,"))?;
    let update_item = MenuItem::with_id(
        app,
        "check-updates",
        "Check for Updates",
        true,
        None::<&str>,
    )?;
    let about_item = MenuItem::with_id(app, "about", "About Cortex", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, Some("CmdOrCtrl+Q"))?;

    #[cfg(target_os = "macos")]
    let app_menu = Menu::default(app)?;
    #[cfg(not(target_os = "macos"))]
    let app_menu = Menu::new(app)?;
    let cortex_menu = Submenu::with_items(
        app,
        "Cortex",
        true,
        &[
            &about_item,
            &update_item,
            &PredefinedMenuItem::separator(app)?,
            &settings_item,
            &show_item,
            &PredefinedMenuItem::separator(app)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::services(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::hide(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::hide_others(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;
    #[cfg(target_os = "macos")]
    app_menu.remove_at(0)?;
    app_menu.insert(&cortex_menu, 0)?;
    #[cfg(target_os = "macos")]
    app.set_menu(app_menu)?;
    #[cfg(not(target_os = "macos"))]
    {
        let window = app
            .get_webview_window(main_window::LABEL)
            .ok_or(tauri::Error::WindowNotFound)?;
        window.set_menu(app_menu)?;
        // On Windows and Linux the menu would be a bar of its own under the title bar, with only
        // "Cortex" in it: the title said twice. It stays attached, hidden, so its shortcuts
        // (Ctrl+, for Settings, Ctrl+Q to quit) keep working where the platform allows; the same
        // items are in the tray menu and the window's own menu at the top left.
        window.hide_menu()?;
    }

    let menu = Menu::with_items(
        app,
        &[
            &show_item,
            &settings_item,
            &update_item,
            &about_item,
            &quit_item,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ICON_ID)
        .icon(icon_for_theme(system_theme(app)))
        .icon_as_template(TRAY_ICON_IS_TEMPLATE)
        .menu(&menu)
        .build(app)?;

    let app_handle = app.clone();
    // App-level listeners receive tray events too; registering once avoids running twice.
    app.on_menu_event(move |_app, event| match event.id.as_ref() {
        "show" => {
            if let Err(e) = main_window::show(&app_handle) {
                error!("could not show main window: {e}");
            }
        }
        "settings" => {
            if let Err(e) = settings::window::show(&app_handle) {
                error!("could not show settings: {e}");
            }
        }
        "check-updates" => {
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                updater::check_and_prompt(&handle).await;
            });
        }
        "about" => show_about(&app_handle),
        "quit" => app_handle.exit(0),
        _ => {}
    });
    Ok(())
}

// Version comes from tauri.conf.json via package_info.
/// The app menu's actions from the window's own menu (Vibe's menu at the top left): the same
/// as the tray's.
#[tauri::command]
pub fn app_menu_action(app: AppHandle, action: String) -> Result<(), String> {
    match action.as_str() {
        "check-updates" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                updater::check_and_prompt(&handle).await;
            });
        }
        "about" => show_about(&app),
        "quit" => app.exit(0),
        other => return Err(format!("unknown action {other}")),
    }
    Ok(())
}

fn show_about(app: &AppHandle) {
    let info = app.package_info();
    branded::show(
        app,
        "About Cortex",
        format!("Version {}", info.version),
        None,
        branded::Layout::Native,
        |_| {},
    );
}
