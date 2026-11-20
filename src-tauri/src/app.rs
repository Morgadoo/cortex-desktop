//! Startup: the command-line modes that run without a window, then the Tauri app, its plugins
//! and state, the main window, shortcuts and background services, and what happens at exit.
//!
//! The order of the steps in [`setup`] matters where a comment says so.

use log::{error, info, warn};
use tauri::{App, AppHandle, Emitter, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::settings::{self, load_device_settings, DeviceSettings};
use crate::tools::{self, registry::ToolRegistry};
use crate::{
    account, browser_login, chat, downloads, main_window, quick_ask, reporting, reset, theme, tray,
    updater, webview,
};

pub fn run() {
    before_any_window();
    // The guard must outlive the event loop to prevent premature client shutdown.
    let _sentry = reporting::init();
    tauri::Builder::default()
        .invoke_handler(|invoke| {
            let app = invoke.message.webview().app_handle().clone();
            if updater::is_required(&app) {
                invoke.resolver.reject("Update Cortex to continue");
                return true;
            }
            crate::handlers::handler(invoke)
        })
        // Must be registered before any other plugin: it intercepts a second launch and
        // forwards its arguments to the running instance, which is how Linux and Windows
        // deliver deep links.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Err(e) = main_window::show(app) {
                warn!("failed to show main window on second launch: {e}");
            }
        }))
        .plugin(log_plugin())
        // Mini apps the assistant builds run from here, in a sandboxed frame with their own CSP.
        .register_uri_scheme_protocol("cortexapp", |ctx, request| {
            serve_app_uri(ctx.app_handle(), &request)
        })
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(setup)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(on_event);
}

/// What runs before Tauri: moving and unlocking data, and the command-line modes, which exit.
fn before_any_window() {
    // Lumo Desktop's folders move to the new identifier before anything opens them.
    crate::rename_migration::migrate_folders();
    // Encrypted data is unlocked with the keychain's key before anything reads it.
    crate::chat::at_rest::unlock_at_start();
    // The key that signs the tools the assistant wrote, when there are any.
    if let Some(config_dir) = crate::reporting::early_config_dir() {
        crate::chat::script_tools::unlock_signing(&config_dir);
        // Trial copies of the data that a crash or a quit left behind.
        crate::chat::evals::sweep_trials(&config_dir);
    }
    let modes: [fn() -> Option<i32>; 5] = [
        // `-p "question"`: answer on standard output and exit, without a window.
        crate::chat::headless::maybe_run,
        // `chat`: a conversation in the terminal, saved to the app's history.
        crate::chat::terminal_chat::maybe_run,
        // `eval <suite>`: run an evaluation suite and print its score.
        crate::chat::evals::maybe_run,
        // `config schema | check | path`.
        crate::chat::config_cli::maybe_run,
        crate::chat::config_cli::startup_check,
    ];
    for mode in modes {
        if let Some(code) = mode() {
            std::process::exit(code);
        }
    }
}

fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_log::Builder::new()
        .max_file_size(5_000_000)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
        .level(if cfg!(debug_assertions) {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        // Web search parses whole result pages; their parser's debug output is noise.
        .level_for("html5ever", log::LevelFilter::Warn)
        .level_for("selectors", log::LevelFilter::Warn)
        .build()
}

/// `cortexapp://`: emoji images, files from code runs, and canvases and mini apps.
fn serve_app_uri(
    app: &AppHandle,
    request: &tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let failed = || {
        tauri::http::Response::builder()
            .status(500)
            .body(Vec::new())
            .unwrap_or_default()
    };
    // Fluent emoji images, from the pack kept in the local data folder.
    if let Some(rest) = request.uri().path().strip_prefix("/emoji/") {
        return match app.path().app_local_data_dir() {
            Ok(root) => chat::emoji_pack::serve(&root, rest),
            Err(_) => failed(),
        };
    }
    match crate::settings::get_config_dir(app) {
        Ok(dir) => match request.uri().path().strip_prefix("/sandbox/") {
            Some(rest) => chat::sandbox::serve_range(
                &dir,
                rest,
                request.headers().get("range").and_then(|v| v.to_str().ok()),
            ),
            None => chat::canvas::serve(&dir, request.uri().path()),
        },
        Err(_) => failed(),
    }
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    app.manage(updater::UpdateState::default());
    app.manage(downloads::Downloads::default());
    app.manage(crate::dictation::Dictation::default());
    app.manage(chat::recorder::Recorder::default());
    if let Ok(dir) = app.path().app_local_data_dir() {
        chat::local_models::init(&dir);
    }
    // First line of every launch: the log appends across sessions, and a bug report is only
    // useful with the version and platform it came from.
    info!(
        "Cortex {} starting on {} {} ({})",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    if let Err(message) = reset::apply_pending_reset(app.handle()) {
        report_failed_reset(app, message);
        return Ok(());
    }

    manage_window_state(app);
    // Before anything reads the keychain: chat setup starts the local API in the background, and
    // its key cannot load until the keychain knows where withdrawn credentials are kept.
    match settings::get_config_dir(app.handle()) {
        Ok(config_dir) => tools::auth::set_pending_deletes_dir(config_dir),
        Err(e) => error!("connector secrets will not load: {e}"),
    }
    chat::commands::setup(app);
    #[cfg(feature = "hosted")]
    {
        app.manage(crate::skills::SkillCache::new(
            app.path().app_cache_dir()?.join("skill-runtime"),
            app.state::<account::ActiveAccount>().get(),
        ));
        app.manage(crate::schedules::coordinator::ScheduleCoordinator::start(
            app.handle(),
        ));
    }
    handle_deep_links(app);
    set_up_tools(app);

    let settings = load_device_settings(app.handle()).unwrap_or_else(|e| {
        warn!("could not load settings, using defaults: {e}");
        DeviceSettings::default()
    });
    set_launch_at_startup(app, &settings)?;

    if crate::HOSTED_WEB_APP || crate::e2e_testing_enabled() {
        app.add_capability(main_window::web_app_capability(crate::LUMO_URL)?)?;
    }
    main_window::build(app)?;
    handle_shortcuts(app)?;
    #[cfg(target_os = "macos")]
    {
        crate::unhandled_keys::install();
        app.set_activation_policy(tauri::ActivationPolicy::Regular);
    }
    register_quick_ask_shortcut(app, &settings)?;
    start_background_services(app.handle())?;
    Ok(())
}

/// A reset asked for at the last quit could not be done: retry at the next launch, or give up.
fn report_failed_reset(app: &App, message: String) {
    error!("local configuration reset failed: {message}");
    let handle = app.handle().clone();
    app.dialog()
        .message(format!("Could not reset local configuration: {message}\n\nRetry later closes Cortex and retries the reset on the next launch. Cancel reset reopens Cortex without retrying. Changes already made will remain."))
        .title("Reset failed")
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            "Retry later".into(),
            "Cancel reset".into(),
        ))
        .show(move |retry_later| {
            if retry_later {
                handle.exit(1);
                return;
            }
            match reset::cancel_pending_reset(&handle) {
                Ok(()) => handle.restart(),
                Err(message) => {
                    error!("could not cancel local configuration reset: {message}");
                    let exit_handle = handle.clone();
                    handle
                        .dialog()
                        .message(message)
                        .title("Could not cancel reset")
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                        .show(move |_| exit_handle.exit(1));
                }
            }
        });
}

/// The state the windows and the account share, and which account the app acts as.
fn manage_window_state(app: &App) {
    app.manage(browser_login::PendingHandoff::default());
    app.manage(webview::LoginPage::default());
    app.manage(settings::window::PendingSettingsTab::default());
    app.manage(theme::WebTheme::default());
    app.manage(quick_ask::PendingQuickAsk::default());
    app.manage(account::ActiveAccount::default());
    app.manage(crate::feature_flags::DesktopFeatureFlags::default());
    app.manage(tools::approvals::PendingApprovals::default());
    if crate::e2e_testing_enabled() {
        account::seed_e2e_account(&app.state::<account::ActiveAccount>());
    } else if !crate::HOSTED_WEB_APP {
        account::seed_local_account(&app.state::<account::ActiveAccount>());
    }
}

/// Links into the app (`cortex://new?prompt=…`): kept until the page takes them, and announced in
/// case it is already open. Other links are OAuth callbacks or the hosted sign-in's.
fn handle_deep_links(app: &App) {
    use tauri_plugin_deep_link::DeepLinkExt;
    app.manage(chat::links::PendingLink::default());
    let open_link = |handle: &AppHandle, link: chat::links::Link| {
        handle.state::<chat::links::PendingLink>().put(link);
        if let Err(e) = main_window::show(handle) {
            warn!("could not show the window for a link: {e}");
        }
        let _ = handle.emit("chat://link", ());
    };
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            if let Some(link) = chat::links::parse(&url) {
                open_link(&handle, link);
            } else if !tools::oauth::handle_deep_link(&url) {
                browser_login::handle_deep_link(&handle, std::slice::from_ref(&url));
            }
        }
    });
    // A link that launched the app.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            if let Some(link) = chat::links::parse(&url) {
                app.state::<chat::links::PendingLink>().put(link);
            }
        }
    }
}

/// The connector registry, its approval gate, and the active account's connectors.
fn set_up_tools(app: &App) {
    let registry = ToolRegistry::new();
    registry.report_failures_to(crate::reporting::approval_failure);
    registry.set_approval_gate(std::sync::Arc::new(
        tools::approvals::InteractiveApprovalGate::new(app.handle().clone()),
    ));
    registry.set_host(std::sync::Arc::new(tools::AccountStateFile::new(
        app.handle().clone(),
    )));
    app.manage(registry);
    app.manage(tools::auth::SecureStorageStatus {
        available: tauri::async_runtime::block_on(tools::auth::secure_storage_available()),
    });
    // Blocks so the registry is populated before any command can read it.
    let account = app.state::<account::ActiveAccount>().get();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(tools::swap_account(&handle, account));
}

fn set_launch_at_startup(
    app: &App,
    settings: &DeviceSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    app.handle().plugin(tauri_plugin_autostart::init(
        MacosLauncher::LaunchAgent,
        None,
    ))?;
    let autostart = app.autolaunch();
    let _ = if settings.launch_at_startup {
        autostart.enable()
    } else {
        autostart.disable()
    };
    Ok(())
}

/// What the global shortcuts do: hold-to-dictate, and the one that opens Quick Ask.
fn handle_shortcuts(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let app_handle = app.handle().clone();
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |_app, shortcut, event| {
                let dictation = crate::dictation::is_shortcut(&app_handle, shortcut);
                match event.state() {
                    ShortcutState::Pressed if dictation => crate::dictation::pressed(&app_handle),
                    ShortcutState::Released if dictation => crate::dictation::released(&app_handle),
                    ShortcutState::Pressed => {
                        if let Err(e) = quick_ask::window::show(&app_handle) {
                            error!("could not show search window: {e}");
                        }
                    }
                    ShortcutState::Released => {}
                }
            })
            .build(),
    )?;
    Ok(())
}

fn register_quick_ask_shortcut(
    app: &App,
    settings: &DeviceSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    if settings.search_shortcut_enabled {
        let shortcut = settings
            .search_shortcut
            .parse::<Shortcut>()
            .unwrap_or_else(|_| Shortcut::new(Some(Modifiers::CONTROL), Code::KeyL));
        app.handle().global_shortcut().register(shortcut)?;
    }
    Ok(())
}

/// What runs alongside the window: dictation, sync, share links, chat platforms, the tray and
/// the update check.
fn start_background_services(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    crate::dictation::startup(app);
    chat::sync::start(app.clone());
    chat::share::start(app);
    chat::telegram::start(app);
    chat::slack::start(app);
    chat::discord::start(app);
    tray::create(app)?;
    updater::startup(app);
    Ok(())
}

fn on_event(app: &AppHandle, event: tauri::RunEvent) {
    // Used on macOS and in hosted builds.
    let _ = &app;
    #[cfg(target_os = "macos")]
    if matches!(event, tauri::RunEvent::Reopen { .. }) {
        if let Err(e) = main_window::show(app) {
            warn!("failed to show main window on reopen: {e}");
        }
    }
    if matches!(event, tauri::RunEvent::Exit) {
        // Background commands from coding sessions end with the app.
        chat::terminal::stop_all();
        #[cfg(feature = "hosted")]
        {
            if let Some(cache) = app.try_state::<crate::skills::SkillCache>() {
                cache.activate(None);
            }
            if let Some(coordinator) =
                app.try_state::<crate::schedules::coordinator::ScheduleCoordinator>()
            {
                coordinator.shutdown();
            }
        }
    }
}
