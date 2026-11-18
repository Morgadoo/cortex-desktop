use log::{error, info, warn};
use tauri::{Manager, WebviewUrl};

/// Bundled sign-in page, shown whenever there is no session to load the web app with.
pub const LOGIN_PAGE: &str = "login.html";

/// The native chat page the main window opens on.
pub const APP_PAGE: &str = "index.html";

/// Where the main window starts.
pub fn main_window_url() -> WebviewUrl {
    if crate::e2e_testing_enabled() {
        info!(
            "LUMO_E2E_TESTING set: loading e2e_testing_stub.html instead of {}",
            crate::LUMO_URL
        );
        WebviewUrl::App("e2e_testing_stub.html".into())
    } else {
        WebviewUrl::App(if crate::HOSTED_WEB_APP { LOGIN_PAGE } else { APP_PAGE }.into())
    }
}

/// URL of the sign-in page, captured at startup so the window can be returned to it.
#[derive(Default)]
pub struct LoginPage(std::sync::Mutex<Option<url::Url>>);

/// Record the sign-in page the window starts on, then load the web app over it.
///
/// The bundled-asset URL differs by platform and between dev and release builds, and WebView2
/// answers `about:blank` until its first navigation commits, so the URL is taken from that
/// navigation rather than read off the window. Recording happens once, which is also what keeps
/// the bounce back to sign-in from resuming again.
fn remember_login_page(app_handle: &tauri::AppHandle, url: &url::Url) {
    let login_page = app_handle.state::<LoginPage>();
    let Ok(mut slot) = login_page.0.lock() else {
        error!("sign-in page lock poisoned");
        return;
    };
    if slot.is_some() {
        return;
    }
    info!("sign-in page is at {url}");
    *slot = Some(url.clone());
    drop(slot);

    resume_web_app(app_handle);
}

fn resume_web_app(app_handle: &tauri::AppHandle) {
    // The native chat page is the app now; there is no hosted web app to resume.
    if crate::e2e_testing_enabled() || !crate::HOSTED_WEB_APP {
        return;
    }

    let url = match crate::LUMO_URL.parse::<url::Url>() {
        Ok(url) => url,
        Err(e) => {
            error!("LUMO_URL is not a URL, staying on sign-in: {e}");
            return;
        }
    };

    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(webview) = app_handle.get_webview(crate::main_window::LABEL) {
            if let Err(e) = webview.navigate(url) {
                error!("could not load the web app, staying on sign-in: {e}");
            }
        }
    });
}

/// Return the main window to the sign-in page.
fn show_login_page(app_handle: &tauri::AppHandle) {
    let Some(url) = app_handle
        .state::<LoginPage>()
        .0
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
    else {
        warn!("sign-in page URL unknown, cannot return to it");
        return;
    };

    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(webview) = app_handle.get_webview(crate::main_window::LABEL) {
            if let Err(e) = webview.navigate(url) {
                error!("could not return to sign-in: {e}");
            }
        }
    });
}

/// Whether `url` is one of the application's own bundled pages.
///
/// `tauri://localhost` on macOS, `http://tauri.localhost` on Windows, and a loopback dev server
/// under `tauri dev`. Loopback is accepted only in debug builds.
fn is_bundled_page(url: &url::Url) -> bool {
    if url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") {
        return true;
    }

    cfg!(debug_assertions)
        && matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("[::1]")
        )
}

#[derive(Debug, PartialEq)]
enum Navigation {
    Follow,
    SignIn,
    SwitchAccount,
    Browser,
    Ignore,
}

/// Whether `url` is on the account origin for this build's environment.
fn is_account(url: &url::Url) -> bool {
    super::browser_login::account_url(crate::LUMO_URL)
        .ok()
        .and_then(|account| account.parse::<url::Url>().ok())
        .is_some_and(|account| account.origin() == url.origin())
}

/// Where the web app loads its artifact iframe from, on the lumo-api origin.
const ARTIFACT_SHELL_PATH: &str = "/lumo/v1/artifact-shell";

fn is_lumo_api(url: &url::Url) -> bool {
    crate::LUMO_URL
        .parse::<url::Url>()
        .ok()
        .and_then(|lumo| lumo.host_str().map(str::to_owned))
        .and_then(|host| host.strip_prefix("lumo.").map(str::to_owned))
        .and_then(|domain| {
            format!("https://lumo-api.{domain}")
                .parse::<url::Url>()
                .ok()
        })
        .is_some_and(|api| api.origin() == url.origin())
}

/// Whether an account URL is the web app reporting that the session has ended.
///
/// Sign-out redirects to `account/switch?reason=signout`, and a session that dies redirects to
/// `account/authorize?reason=session-expired`. Account links the user follows themselves — "Add
/// another account", "Switch account" — carry no `reason`, which is what separates the two.
fn reports_session_loss(url: &url::Url) -> bool {
    url.query_pairs()
        .any(|(key, value)| key == "reason" && (value == "signout" || value == "session-expired"))
}

/// "Switch or add account" navigates to `account/authorize?app=proton-lumo&state=…&v=2&t=1&pv=3`.
/// Matched on the last segment, since account prefixes a session slot as `/u/<n>/`.
fn requests_another_account(url: &url::Url) -> bool {
    url.path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .is_some_and(|last| matches!(last, "authorize" | "switch" | "login"))
}

fn classify(url: &url::Url) -> Navigation {
    if crate::e2e_testing_enabled() || is_bundled_page(url) {
        return Navigation::Follow;
    }

    let Ok(lumo) = crate::LUMO_URL.parse::<url::Url>() else {
        error!("LUMO_URL is not a URL, refusing {}", loggable(url));
        return Navigation::Ignore;
    };

    if lumo.origin() == url.origin() {
        // Guest mode is not offered on desktop
        return if url.path().starts_with("/guest") {
            Navigation::SignIn
        } else {
            Navigation::Follow
        };
    }

    // lumo-api hosts model-generated artifact script, so the window only follows it to load the
    // artifact shell where iframe navigations still reach this handler (Linux).
    if is_lumo_api(url) {
        return if url.path() == ARTIFACT_SHELL_PATH {
            Navigation::Follow
        } else {
            Navigation::Ignore
        };
    }

    if is_account(url) {
        return if reports_session_loss(url) {
            Navigation::SignIn
        } else if requests_another_account(url) {
            Navigation::SwitchAccount
        } else {
            Navigation::Browser
        };
    }

    if matches!(url.scheme(), "http" | "https") {
        Navigation::Browser
    } else {
        Navigation::Ignore
    }
}

/// Decide whether the main window may follow `url`, acting on the refusal when it may not.
///
/// Registered as the window's navigation handler.
pub fn handle_navigation(app_handle: &tauri::AppHandle, url: &url::Url) -> bool {
    if super::updater::is_required(app_handle) {
        return false;
    }
    match classify(url) {
        Navigation::Follow => {
            if is_bundled_page(url) {
                remember_login_page(app_handle, url);
            }
            true
        }
        Navigation::SignIn => {
            info!("session ended at {}, returning to sign-in", url.path());
            super::account::clear_on_session_loss(app_handle);
            show_login_page(app_handle);
            false
        }
        Navigation::SwitchAccount => {
            info!("{} requests another account, signing in", url.path());
            super::browser_login::begin_browser_login(app_handle);
            // Under `tauri dev` the `proton-lumo://` callback never arrives, so the handoff needs
            // the sign-in page's "I've signed in" button, which a switch never lands on.
            #[cfg(debug_assertions)]
            show_login_page(app_handle);
            false
        }
        Navigation::Browser => {
            if is_proton(url) {
                open_in_browser(url);
            } else if crate::inapp_browser::is_web(url) && crate::inapp_browser::prefs(app_handle).in_app {
                // Lumo's own browser, when chosen; the system browser is one menu item away.
                if let Err(e) = crate::inapp_browser::open(app_handle, url) {
                    error!("could not open Lumo's browser: {e}");
                    confirm_and_open(app_handle, url.clone());
                }
            } else {
                confirm_and_open(app_handle, url.clone());
            }
            false
        }
        Navigation::Ignore => {
            info!("refused {}", loggable(url));
            false
        }
    }
}

/// Scheme and host only: paths, queries and fragments can carry conversation content or keys.
fn loggable(url: &url::Url) -> String {
    match url.host_str() {
        Some(host) => format!("{}://{host}", url.scheme()),
        None => url.scheme().to_string(),
    }
}

/// Whether `url` is on a Proton domain, or on this build's environment domain.
fn is_proton(url: &url::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let environment = crate::LUMO_URL
        .parse::<url::Url>()
        .ok()
        .and_then(|lumo| lumo.host_str().map(str::to_owned))
        .and_then(|host| host.strip_prefix("lumo.").map(str::to_owned));
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    under("proton.me") || environment.as_deref().is_some_and(under)
}

fn open_in_browser(url: &url::Url) {
    info!("opening {} in the browser", loggable(url));
    // Detached, because the navigation handler runs on the UI thread and `open::that`
    // waits for the launcher to exit.
    if let Err(e) = open::that_detached(url.as_str()) {
        // Not the error's text: it can quote the whole URL, query included.
        error!(
            "could not open {} in the browser: {:?}",
            loggable(url),
            e.kind()
        );
    }
}

/// Links can come from model output, so leaving Proton is confirmed, naming the host. Only one
/// prompt is shown at a time; links followed while it is open are dropped.
fn confirm_and_open(app_handle: &tauri::AppHandle, url: url::Url) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PROMPTING: AtomicBool = AtomicBool::new(false);

    if PROMPTING.swap(true, Ordering::SeqCst) {
        info!("refused {} while a link prompt is open", loggable(&url));
        return;
    }
    // Owned by the callback, so the flag clears even if the dialog drops it without calling it.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PROMPTING.store(false, Ordering::SeqCst);
        }
    }
    let reset = Reset;
    let host = url.host_str().unwrap_or_default().to_string();
    let mut shown: String = url.as_str().chars().take(300).collect();
    if shown.len() < url.as_str().len() {
        shown.push('…');
    }
    crate::dialog::branded::show(
        app_handle,
        "Open link in your browser?",
        format!("This link goes to {host}, outside Lumo.\n\n{shown}"),
        Some(("Open", "Cancel")),
        crate::dialog::branded::Layout::IconOnLeft,
        move |accepted| {
            drop(reset);
            if accepted {
                open_in_browser(&url);
            } else {
                info!("did not open {}", loggable(&url));
            }
        },
    );
}

/// Decide what to do with a `target="_blank"` link, which the webviews report as a request for a
/// new window rather than as a navigation.
///
/// Always denies: the only thing the response can grant is a popup webview the app has no use for
pub fn handle_new_window(
    app_handle: &tauri::AppHandle,
    url: &url::Url,
) -> tauri::webview::NewWindowResponse<tauri::Wry> {
    handle_navigation(app_handle, url);
    tauri::webview::NewWindowResponse::Deny
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_str(url: &str) -> Navigation {
        classify(&url.parse().unwrap())
    }

    fn account(path_and_query: &str) -> String {
        format!(
            "{}{path_and_query}",
            super::super::browser_login::account_url(crate::LUMO_URL).unwrap()
        )
    }

    #[test]
    fn the_lumo_origin_and_bundled_pages_are_followed() {
        assert_eq!(classify_str(crate::LUMO_URL), Navigation::Follow);
        assert_eq!(
            classify_str(&format!("{}/login#selector=x&sk=y", crate::LUMO_URL)),
            Navigation::Follow
        );
        assert_eq!(
            classify_str("tauri://localhost/login.html"),
            Navigation::Follow
        );
        assert_eq!(
            classify_str(&format!("{}/#prefill=hello", crate::LUMO_URL)),
            Navigation::Follow
        );
        // `tauri dev` serves bundled pages over loopback rather than the asset protocol.
        let loopback = classify_str("http://127.0.0.1:1430/login.html");
        if cfg!(debug_assertions) {
            assert_eq!(loopback, Navigation::Follow);
        } else {
            assert_eq!(loopback, Navigation::Browser);
        }
    }

    #[test]
    fn the_lumo_api_origin_is_followed() {
        let api = crate::LUMO_URL.replace("//lumo.", "//lumo-api.");
        assert_eq!(
            classify_str(&format!("{api}/lumo/v1/artifact-shell")),
            Navigation::Follow
        );
        assert_eq!(
            classify_str("https://lumo-api.example.com/lumo/v1/artifact-shell"),
            Navigation::Browser
        );
    }

    #[test]
    fn the_window_never_leaves_for_other_lumo_api_pages() {
        let api = crate::LUMO_URL.replace("//lumo.", "//lumo-api.");
        for path in ["/anything", "/lumo/v1/artifact-shell/x", "/", "/lumo/v1/"] {
            assert_eq!(
                classify_str(&format!("{api}{path}")),
                Navigation::Ignore,
                "{path}"
            );
        }
    }

    #[test]
    fn signing_out_and_expiring_return_to_sign_in() {
        assert_eq!(
            classify_str(&account("/switch?product=lumo&reason=signout&type=self")),
            Navigation::SignIn
        );
        assert_eq!(
            classify_str(&account(
                "/authorize?app=proton-lumo&reason=session-expired"
            )),
            Navigation::SignIn
        );
        assert_eq!(
            classify_str(&format!("{}/guest", crate::LUMO_URL)),
            Navigation::SignIn
        );
    }

    #[test]
    fn asking_for_another_account_starts_browser_sign_in() {
        // Captured from the web app.
        assert_eq!(
            classify_str(&account(
                "/authorize?app=proton-lumo&state=hhHouLgIAxGZPHiVeT5B8LNIeRNnstJwR0kA&v=2&t=1&pv=3"
            )),
            Navigation::SwitchAccount
        );
        assert_eq!(
            classify_str(&account("/login?product=lumo&prompt=login")),
            Navigation::SwitchAccount
        );
        assert_eq!(
            classify_str(&account("/switch?product=lumo")),
            Navigation::SwitchAccount
        );
        assert_eq!(
            classify_str(&account("/u/1/switch")),
            Navigation::SwitchAccount
        );
    }

    #[test]
    fn other_account_links_open_in_the_browser() {
        assert_eq!(classify_str(&account("/lumo")), Navigation::Browser);
        assert_eq!(classify_str(&account("/dashboard")), Navigation::Browser);
    }

    #[test]
    fn marketing_links_open_in_the_browser() {
        assert_eq!(classify_str("https://proton.me/lumo"), Navigation::Browser);
        assert_eq!(
            classify_str("https://proton.me/fr/lumo"),
            Navigation::Browser
        );
        assert_eq!(classify_str("https://proton.me"), Navigation::Browser);
    }

    #[test]
    fn non_web_schemes_never_reach_the_browser() {
        assert_eq!(classify_str("file:///etc/passwd"), Navigation::Ignore);
        assert_eq!(
            classify_str("mailto:someone@example.com"),
            Navigation::Ignore
        );
        assert_eq!(classify_str("javascript:alert(1)"), Navigation::Ignore);
    }

    #[test]
    fn logged_urls_keep_only_scheme_and_host() {
        let url = |s: &str| s.parse::<url::Url>().unwrap();
        assert_eq!(
            loggable(&url("https://example.com/a/b?token=1#secret")),
            "https://example.com"
        );
        assert_eq!(loggable(&url("mailto:someone@example.com")), "mailto");
    }

    #[test]
    fn only_links_leaving_proton_need_confirmation() {
        let url = |s: &str| s.parse::<url::Url>().unwrap();
        assert!(is_proton(&url("https://proton.me/lumo")));
        assert!(is_proton(&url("https://drive.proton.me/urls/x#key")));
        assert!(is_proton(&url(crate::LUMO_URL)));
        assert!(!is_proton(&url("https://evil.example/c?d=1")));
        assert!(!is_proton(&url("https://proton.me.evil.example/")));
        assert!(!is_proton(&url("https://notproton.me/")));
    }
}
