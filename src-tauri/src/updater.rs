use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use semver::Version;
use tauri::{AppHandle, Manager, Runtime, Url};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::dialog::loading::Loading;

const BUILD_TIME_MANIFEST_URL: Option<&str> = option_env!("CORTEX_UPDATE_MANIFEST_URL");

/// The GitHub repository releases are published to, as `owner/name`. A fork that publishes its
/// own releases builds with `CORTEX_RELEASES_REPO` set to its repository.
const RELEASES_REPO: &str = match option_env!("CORTEX_RELEASES_REPO") {
    Some(repo) => repo,
    None => "Morgadoo/cortex-desktop",
};

/// Which releases this installation follows (Settings → General).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Published releases only.
    Stable,
    /// Pre-releases as well (`0.6.0-beta.1`), as soon as they are published.
    Preview,
}

impl Default for UpdateChannel {
    /// A preview build follows previews, a stable build stable releases, until the user chooses.
    fn default() -> Self {
        if env!("CARGO_PKG_VERSION").contains('-') {
            Self::Preview
        } else {
            Self::Stable
        }
    }
}

impl UpdateChannel {
    /// The update manifest for the channel. GitHub's `releases/latest` is the newest release
    /// that is not a pre-release; previews come from the rolling `updater` release, whose
    /// `preview.json` the release workflow replaces with every release, stable or not.
    pub fn manifest_url(self) -> String {
        match self {
            Self::Stable => {
                format!("https://github.com/{RELEASES_REPO}/releases/latest/download/latest.json")
            }
            Self::Preview => {
                format!("https://github.com/{RELEASES_REPO}/releases/download/updater/preview.json")
            }
        }
    }
}

#[derive(Default)]
pub struct UpdateState {
    // Remains set until process exit after a mandatory update is detected.
    required: AtomicBool,
    // Held through the check, dialog, and installation, including mandatory-update retries.
    busy: AtomicBool,
}

pub fn is_required<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<UpdateState>()
        .is_some_and(|state| state.required.load(Ordering::SeqCst))
}

// Endpoint precedence: runtime override, build-time override, then the chosen channel's.
fn manifest_url(app: &AppHandle) -> String {
    if let Some(url) = std::env::var("CORTEX_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.is_empty())
    {
        return url;
    }
    if let Some(url) = BUILD_TIME_MANIFEST_URL.filter(|value| !value.is_empty()) {
        return url.to_string();
    }
    crate::settings::get_config_dir(app)
        .map(|dir| crate::reporting::load_prefs(&dir).update_channel)
        .unwrap_or_default()
        .manifest_url()
}

fn mandatory(current: &Version, release: &Version, raw: &serde_json::Value) -> bool {
    let Some(value) = raw.get("minimum_supported_version") else {
        return false;
    };
    let Some(text) = value.as_str() else {
        log::warn!("Ignoring minimum_supported_version: expected a version string");
        return false;
    };
    let minimum = match Version::parse(text) {
        Ok(version) => version,
        Err(error) => {
            log::warn!("Ignoring invalid minimum_supported_version: {error}");
            return false;
        }
    };
    if minimum > *release {
        log::warn!(
            "Ignoring minimum_supported_version {minimum}: exceeds offered release {release}"
        );
        return false;
    }
    *current < minimum
}

async fn check(app: &AppHandle) -> Result<Option<(Update, bool)>, String> {
    let mut builder = app.updater_builder().timeout(Duration::from_secs(15));
    let url = manifest_url(app);
    let parsed: Url = url
        .parse()
        .map_err(|e| format!("Invalid update endpoint {url}: {e}"))?;
    builder = builder
        .endpoints(vec![parsed])
        .map_err(|e| format!("Invalid update endpoint {url}: {e}"))?;

    let update = builder
        .build()
        .map_err(|e| format!("Could not build updater: {e}"))?
        .check()
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;
    update
        .map(|update| {
            let version = Version::parse(&update.version).map_err(|e| e.to_string())?;
            let required = mandatory(&app.package_info().version, &version, &update.raw_json);
            Ok((update, required))
        })
        .transpose()
}

async fn install(app: &AppHandle, mut update: Update) -> Result<(), String> {
    let loading = Loading::show(
        app,
        "Updating Cortex",
        "Downloading and installing the update…",
    );
    update.timeout = Some(Duration::from_secs(600));
    let result = update
        .download_and_install(|_downloaded, _total| {}, || {})
        .await
        .map_err(|e| format!("Update install failed: {e}"));
    loading.finish().await;
    result
}

fn require_update(app: &AppHandle) {
    app.state::<UpdateState>()
        .required
        .store(true, Ordering::SeqCst);
    for window in app.webview_windows().values() {
        let _ = window.set_enabled(false);
        let _ = window.hide();
    }
}

// Use callback dialogs to avoid blocking the application's main thread.
fn prompt(app: &AppHandle, update: Update, required: bool, error: Option<String>) {
    let handle = app.clone();
    let message = match &error {
        Some(error) => format!("{error}\n\nCortex {} is required to continue. Retry the update or quit.", update.version),
        None if required => format!("This version of Cortex is no longer supported. Update to Cortex {} to continue. Cortex will restart to finish installing.", update.version),
        None => format!("Cortex {} is available. Cortex will restart to finish installing.", update.version),
    };
    crate::dialog::branded::show(
        app,
        if required {
            "Update required"
        } else {
            "Update available"
        },
        message,
        Some((
            if error.is_some() { "Retry" } else { "Update" },
            if required { "Quit" } else { "Later" },
        )),
        crate::dialog::branded::Layout::IconOnLeft,
        move |accepted| {
            if !accepted {
                if required {
                    handle.exit(0);
                } else {
                    handle
                        .state::<UpdateState>()
                        .busy
                        .store(false, Ordering::SeqCst);
                }
                return;
            }
            tauri::async_runtime::spawn(async move {
                match install(&handle, update.clone()).await {
                    Ok(()) => handle.restart(),
                    Err(error) if required => prompt(&handle, update, true, Some(error)),
                    Err(error) => {
                        handle
                            .state::<UpdateState>()
                            .busy
                            .store(false, Ordering::SeqCst);
                        show_error(&handle, error);
                    }
                }
            });
        },
    );
}

/// Whether this copy runs in a Flatpak, which keeps it up to date (`flatpak update`): the app's
/// own updater, which replaces its files in place, stands aside.
pub fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

pub async fn check_and_prompt(app: &AppHandle) {
    if in_flatpak() {
        crate::dialog::branded::show(
            app,
            "Updates come from Flatpak",
            "This copy of Cortex was installed with Flatpak, which keeps it up to date. To update \
             it now, run `flatpak update` or use your software center."
                .to_string(),
            None,
            crate::dialog::branded::Layout::IconOnLeft,
            |_| {},
        );
        return;
    }
    if app.state::<UpdateState>().busy.swap(true, Ordering::SeqCst) {
        return;
    }
    run_check(app, false).await;
}

pub fn startup(app: &AppHandle) {
    if crate::e2e_testing_enabled() || in_flatpak() {
        return;
    }
    // The user can turn the check at start off (Settings → General → Privacy).
    if crate::settings::get_config_dir(app)
        .is_ok_and(|dir| !crate::reporting::load_prefs(&dir).update_checks)
    {
        return;
    }
    // Acquire the update guard before spawning to prevent concurrent manual checks.
    if app.state::<UpdateState>().busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_check(&app, true).await;
    });
}

async fn run_check(app: &AppHandle, startup: bool) {
    let state = app.state::<UpdateState>();
    let loading = if startup {
        None
    } else {
        Some(Loading::show(
            app,
            "Checking for updates",
            "Looking for a newer version of Cortex…",
        ))
    };
    let result = if startup {
        tokio::time::timeout(Duration::from_secs(3), check(app))
            .await
            .unwrap_or_else(|_| Err("Startup update check timed out after 3 seconds".into()))
    } else {
        check(app).await
    };
    if let Some(loading) = loading {
        loading.finish().await;
    }
    // Failed checks allow startup so network outages do not prevent application use.
    if startup && !matches!(&result, Ok(Some((_, true)))) {
        if let Err(error) = crate::main_window::show(app) {
            log::warn!("Could not show main window after update check: {error}");
        }
    }
    match result {
        Ok(Some((update, required))) => {
            if required {
                require_update(app);
            }
            prompt(app, update, required, None);
        }
        Ok(None) => {
            state.busy.store(false, Ordering::SeqCst);
            if !startup {
                let version = app.package_info().version.to_string();
                crate::dialog::branded::show(
                    app,
                    "No updates available",
                    format!("Cortex {version} is up to date."),
                    None,
                    crate::dialog::branded::Layout::IconOnLeft,
                    |_| {},
                );
            }
        }
        Err(error) => {
            state.busy.store(false, Ordering::SeqCst);
            if startup {
                log::warn!("{error}");
            } else {
                show_error(app, error);
            }
        }
    }
}

fn show_error(app: &AppHandle, message: String) {
    app.dialog()
        .message(message)
        .title("Update failed")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(current: &str, release: &str, raw: serde_json::Value) -> bool {
        mandatory(
            &Version::parse(current).unwrap(),
            &Version::parse(release).unwrap(),
            &raw,
        )
    }

    #[test]
    fn channels_follow_github_releases() {
        assert_eq!(
            UpdateChannel::Stable.manifest_url(),
            format!("https://github.com/{RELEASES_REPO}/releases/latest/download/latest.json")
        );
        assert!(UpdateChannel::Preview
            .manifest_url()
            .ends_with("/releases/download/updater/preview.json"));
        assert_eq!(
            serde_json::from_str::<UpdateChannel>("\"preview\"").unwrap(),
            UpdateChannel::Preview
        );
        // The build decides until the user chooses: this build's version has no pre-release part.
        let expected = if env!("CARGO_PKG_VERSION").contains('-') {
            UpdateChannel::Preview
        } else {
            UpdateChannel::Stable
        };
        assert_eq!(UpdateChannel::default(), expected);
    }

    #[test]
    fn minimum_version_boundaries() {
        let manifest = json!({"minimum_supported_version": "0.6.0"});
        assert!(policy("0.5.3", "0.6.4", manifest.clone()));
        assert!(!policy("0.6.0", "0.6.4", manifest.clone()));
        assert!(!policy("0.6.3", "0.6.4", manifest.clone()));
        assert!(!policy("0.6.4", "0.6.4", manifest.clone()));
        assert!(!policy("0.7.0", "0.6.4", manifest));
    }

    #[test]
    fn old_manifests_remain_optional() {
        assert!(!policy("0.5.3", "0.6.4", json!({})));
    }

    #[test]
    fn invalid_or_unreachable_minimum_keeps_update_optional() {
        for value in [
            json!(null),
            json!(123),
            json!(false),
            json!([]),
            json!({}),
            json!(""),
            json!("invalid"),
            json!("0.7.0"),
        ] {
            assert!(!policy(
                "0.5.3",
                "0.6.4",
                json!({"minimum_supported_version": value})
            ));
        }
    }

    #[test]
    fn prerelease_is_below_stable_minimum() {
        assert!(policy(
            "0.6.0-beta.1",
            "0.6.4",
            json!({"minimum_supported_version": "0.6.0"})
        ));
    }
}
