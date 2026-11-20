use log::warn;
use tauri::{App, AppHandle, Manager, WebviewWindow};

use crate::{downloads, settings, tray, updater, webview};

pub const LABEL: &str = "cortex-window";

/// Refuses a command called from any webview but the main window's, with `refusal` as the
/// reason. Commands that act for the signed-in page (exports, downloads, schedules) use it, so a
/// satellite window or an embedded frame cannot.
pub fn require(label: &str, refusal: &str) -> Result<(), String> {
    if label == LABEL {
        Ok(())
    } else {
        Err(refusal.to_string())
    }
}

const BUNDLED_PAGES_CAPABILITY: &str = include_str!("../capabilities/default.json");

/// The bundled pages' permissions, granted to the hosted web app only at `lumo_url`'s exact
/// origin and only in the main window. A release build therefore trusts production alone, not
/// every Cortex environment.
pub fn web_app_capability(lumo_url: &str) -> Result<String, String> {
    use tauri::utils::acl::capability::{Capability, CapabilityRemote};

    let origin = url::Url::parse(lumo_url)
        .map_err(|e| format!("LUMO_URL is not a URL: {e}"))?
        .origin();
    if !origin.is_tuple() {
        return Err(format!("LUMO_URL has no origin: {lumo_url}"));
    }
    let mut capability: Capability = serde_json::from_str(BUNDLED_PAGES_CAPABILITY)
        .map_err(|e| format!("bundled capability is invalid: {e}"))?;
    capability.identifier = "web-app".into();
    capability.description = "The hosted Lumo web app in the main window.".into();
    capability.local = false;
    capability.remote = Some(CapabilityRemote {
        urls: vec![origin.ascii_serialization()],
    });
    capability.windows = vec![LABEL.into()];
    capability.webviews = vec![];
    serde_json::to_string(&capability).map_err(|e| e.to_string())
}

pub fn build(app: &App) -> tauri::Result<WebviewWindow> {
    let e2e = crate::e2e_testing_enabled();
    #[cfg_attr(not(feature = "hosted"), allow(unused_mut))]
    let mut builder = tauri::WebviewWindowBuilder::new(app, LABEL, webview::main_window_url())
        .title("Cortex")
        // The bundled chat page shows itself; only the hosted web app used to reveal the window.
        .visible(e2e || !crate::HOSTED_WEB_APP)
        .inner_size(1280.0, 840.0)
        .min_inner_size(720.0, 520.0)
        .prevent_overflow()
        .center()
        .initialization_script(format!(
            "window.lumoDesktopVersion = {};",
            serde_json::to_string(&app.package_info().version.to_string())?,
        ));
    // The bridges were written for the hosted web app. The native chat page talks to Rust
    // directly, and the account bridge would sign its local profile out, so only the e2e stub,
    // which still exercises the bridge contract, gets them.
    #[cfg(feature = "hosted")]
    if e2e || crate::HOSTED_WEB_APP {
        for script in [
            include_str!("../dist/scripts/connectors.js"),
            include_str!("../dist/scripts/settings-hotkey.js"),
            include_str!("../dist/scripts/account-identity.js"),
            include_str!("../dist/scripts/schedules.js"),
            include_str!("../dist/scripts/notifications.js"),
            include_str!("../dist/scripts/skills.js"),
            include_str!("../dist/scripts/quick-ask.js"),
            include_str!("../dist/scripts/download-feedback.js"),
            include_str!("../dist/scripts/theme-reporter.js"),
        ] {
            builder = builder.initialization_script(script);
        }
    }
    let window = builder
        .on_download(downloads::on_download)
        .disable_drag_drop_handler()
        .on_navigation({
            let app_handle = app.handle().clone();
            move |url| webview::handle_navigation(&app_handle, url)
        })
        .on_new_window({
            let app_handle = app.handle().clone();
            move |url, _features| webview::handle_new_window(&app_handle, &url)
        })
        .build()?;

    #[cfg(target_os = "linux")]
    allow_microphone(&window);
    #[cfg(target_os = "macos")]
    crate::subframe_navigation::install(&window);
    #[cfg(target_os = "macos")]
    crate::microphone::install(&window);

    let app_handle = app.handle().clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(&app_handle);
        }
        tauri::WindowEvent::Resized(_) => {
            if let Err(e) = settings::window::sync_bounds(&app_handle) {
                warn!("failed to resize settings overlay: {e}");
            }
        }
        tauri::WindowEvent::ThemeChanged(_) => tray::sync_icon_for_theme(&app_handle),
        _ => {}
    });

    Ok(window)
}

/// WebKitGTK denies getUserMedia unless the permission-request signal is answered, and wry never
/// connects it. Audio only, and only while the page is the web app or the native chat page.
#[cfg(target_os = "linux")]
pub(crate) fn allow_microphone(window: &WebviewWindow) {
    use webkit2gtk::glib::prelude::*;
    use webkit2gtk::{PermissionRequestExt, UserMediaPermissionRequestExt, WebViewExt as _};

    // Under `tauri dev` the page comes from the CLI's own server, not from tauri://localhost.
    let dev_url = window.config().build.dev_url.clone();
    if let Err(e) = window.with_webview(move |webview| {
        // getUserMedia only exists with media streams on, which WebKitGTK leaves off.
        if let Some(settings) = webview.inner().settings() {
            use webkit2gtk::SettingsExt as _;
            settings.set_enable_media_stream(true);
        }
        webview
            .inner()
            .connect_permission_request(move |view, request| {
                let Some(media) = request.downcast_ref::<webkit2gtk::UserMediaPermissionRequest>()
                else {
                    return false;
                };
                let on_web_app = view
                    .uri()
                    .and_then(|uri| uri.parse::<url::Url>().ok())
                    .is_some_and(|url| crate::microphone::is_allowed_in(&url, dev_url.as_ref()));
                if on_web_app && media.is_for_audio_device() && !media.is_for_video_device() {
                    media.allow();
                } else {
                    warn!(
                        "refused a media capture request from {:?} (audio {}, video {})",
                        view.uri(),
                        media.is_for_audio_device(),
                        media.is_for_video_device()
                    );
                    media.deny();
                }
                true
            });
    }) {
        warn!("could not hook microphone permissions: {e}");
    }
}

pub fn show(app_handle: &AppHandle) -> Result<(), String> {
    if updater::is_required(app_handle) {
        return Ok(());
    }
    if let Some(window) = app_handle.get_window(LABEL) {
        #[cfg(target_os = "macos")]
        app_handle
            .set_dock_visibility(true)
            .map_err(|e| e.to_string())?;

        window.show().map_err(|e| e.to_string())?;
        // GTK shows every part of a window as it appears, the hidden menu bar included; it stays
        // hidden (see tray.rs), its shortcuts still working.
        #[cfg(target_os = "linux")]
        window.hide_menu().map_err(|e| e.to_string())?;
        window.unminimize().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Closing the window keeps Cortex running in the tray; on macOS the Dock icon goes with it.
pub(crate) fn hide(app_handle: &AppHandle) {
    let Some(window) = app_handle.get_window(LABEL) else {
        return;
    };
    if let Err(e) = window.hide() {
        warn!("failed to hide main window: {e}");
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = AppHandle::hide(app_handle) {
            warn!("failed to hide app: {e}");
        }
        if let Err(e) = app_handle.set_dock_visibility(false) {
            warn!("failed to hide Dock icon: {e}");
        }
    }
}

/// Show or hide one of the webviews stacked in the main window: the web app or the settings
/// overlay.
pub fn set_page_visible(webview: &tauri::Webview, visible: bool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // with_webview runs GTK operations on the UI thread. Update the
        // recursive-show protection and visibility together to avoid a race.
        webview
            .with_webview(move |native| set_linux_page_visible(&native.inner(), visible))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let result = if visible {
            webview.show()
        } else {
            webview.hide()
        };
        result.map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "linux")]
fn set_linux_page_visible(widget: &impl gtk::glib::IsA<gtk::Widget>, visible: bool) {
    use gtk::prelude::WidgetExt;

    // Likely an upstream Tauri/Tao visibility bug: GTK show_all() on the
    // parent can reveal a webview hidden through Tauri. Protect the hidden
    // page regardless of which restore/activation path shows the parent.
    // Revisit if upstream starts preserving child visibility.
    widget.set_no_show_all(!visible);
    if visible {
        widget.show_all();
    } else {
        widget.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::{web_app_capability, LABEL};
    use tauri::utils::acl::capability::Capability;

    fn capability(lumo_url: &str) -> Capability {
        serde_json::from_str(&web_app_capability(lumo_url).unwrap()).unwrap()
    }

    #[test]
    fn web_app_is_trusted_only_at_the_lumo_url_origin() {
        let capability = capability("https://lumo.proton.me/u/0/chat?x=1");
        let remote = capability.remote.expect("remote origins");
        assert_eq!(remote.urls, ["https://lumo.proton.me"]);
        assert!(!capability.local);
        assert_eq!(capability.windows, [LABEL]);
    }

    #[test]
    fn web_app_gets_the_bundled_pages_permissions() {
        let bundled: Capability = serde_json::from_str(super::BUNDLED_PAGES_CAPABILITY).unwrap();
        assert!(
            bundled.remote.is_none(),
            "bundled pages must not grant remote origins"
        );
        assert_eq!(
            capability("https://lumo.proton.me").permissions,
            bundled.permissions
        );
    }

    #[test]
    fn a_lumo_url_without_an_origin_is_rejected() {
        assert!(web_app_capability("not a url").is_err());
        assert!(web_app_capability("data:text/html,hi").is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_page_visibility_tests {
    use super::set_linux_page_visible;
    use gtk::prelude::*;

    #[test]
    #[ignore = "requires a Linux display; run under xvfb-run with --ignored"]
    fn restoring_parent_preserves_hidden_page() {
        gtk::init().expect("GTK requires a display");
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let main_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let settings_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.pack_start(&main_page, true, true, 0);
        container.pack_start(&settings_page, true, true, 0);
        window.add(&container);
        window.show_all();

        set_linux_page_visible(&main_page, false);
        set_linux_page_visible(&settings_page, true);
        window.hide();
        window.show_all();
        assert!(!main_page.is_visible());
        assert!(settings_page.is_visible());

        set_linux_page_visible(&settings_page, false);
        set_linux_page_visible(&main_page, true);
        window.hide();
        window.show_all();
        assert!(main_page.is_visible());
        assert!(!settings_page.is_visible());
        window.close();
    }
}
