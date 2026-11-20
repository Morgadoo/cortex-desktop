use log::{error, info, warn};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::settings::{self, load_device_settings};
use crate::tools::{self, registry::ToolRegistry};
use crate::{
    account, browser_login, chat, downloads, main_window, quick_ask, reporting, reset, theme, tray,
    updater, webview,
};

pub fn run() {
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
    // `-p "question"`: answer on standard output and exit, without a window (Vibe's headless run).
    if let Some(code) = crate::chat::headless::maybe_run() {
        std::process::exit(code);
    }
    // `config schema | check | path` (Vibe's `vibe config schema`).
    // `chat`: a conversation in the terminal (Vibe Code's CLI), saved to the app's history.
    if let Some(code) = crate::chat::terminal_chat::maybe_run() {
        std::process::exit(code);
    }
    // `eval <suite>`: run an evaluation suite and print its score.
    if let Some(code) = crate::chat::evals::maybe_run() {
        std::process::exit(code);
    }
    if let Some(code) = crate::chat::config_cli::maybe_run() {
        std::process::exit(code);
    }
    if let Some(code) = crate::chat::config_cli::startup_check() {
        std::process::exit(code);
    }
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
        .plugin(
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
                .build(),
        )
        // Mini apps the assistant builds run from here, in a sandboxed frame with their own CSP.
        .register_uri_scheme_protocol("cortexapp", |ctx, request| {
            // Fluent emoji images, from the pack kept in the local data folder.
            if let Some(rest) = request.uri().path().strip_prefix("/emoji/") {
                use tauri::Manager as _;
                return match ctx.app_handle().path().app_local_data_dir() {
                    Ok(root) => chat::emoji_pack::serve(&root, rest),
                    Err(_) => tauri::http::Response::builder().status(500).body(Vec::new()).unwrap_or_default(),
                };
            }
            match crate::settings::get_config_dir(ctx.app_handle()) {
                Ok(dir) => match request.uri().path().strip_prefix("/sandbox/") {
                    Some(rest) => chat::sandbox::serve_range(&dir, rest, request.headers().get("range").and_then(|v| v.to_str().ok())),
                    None => chat::canvas::serve(&dir, request.uri().path()),
                },
                Err(_) => tauri::http::Response::builder().status(500).body(Vec::new()).unwrap_or_default(),
            }
        })
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(updater::UpdateState::default());
            app.manage(downloads::Downloads::default());
            app.manage(crate::dictation::Dictation::default());
            app.manage(chat::recorder::Recorder::default());
            if let Ok(dir) = app.path().app_local_data_dir() {
                chat::local_models::init(&dir);
            }
            // First line of every launch: the log appends across sessions, and a bug report is
            // only useful with the version and platform it came from.
            info!(
                "Cortex {} starting on {} {} ({})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                if cfg!(debug_assertions) { "debug" } else { "release" }
            );

            if let Err(message) = reset::apply_pending_reset(app.handle()) {
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
                                handle.dialog()
                                    .message(message)
                                    .title("Could not cancel reset")
                                    .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                                    .show(move |_| exit_handle.exit(1));
                            }
                        }
                    });
                return Ok(());
            }

            app.manage(browser_login::PendingHandoff::default());
            app.manage(webview::LoginPage::default());
            app.manage(settings::window::PendingSettingsTab::default());
            app.manage(theme::WebTheme::default());
            app.manage(quick_ask::PendingQuickAsk::default());
            app.manage(account::ActiveAccount::default());
            app.manage(crate::feature_flags::DesktopFeatureFlags::default());
            if crate::e2e_testing_enabled() {
                account::seed_e2e_account(&app.state::<account::ActiveAccount>());
            } else if !crate::HOSTED_WEB_APP {
                account::seed_local_account(&app.state::<account::ActiveAccount>());
            }
            // Before anything reads the keychain: chat setup starts the local API in the background,
            // and its key cannot load until the keychain knows where withdrawn credentials are kept.
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

            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.manage(chat::links::PendingLink::default());
                // Links into the app (cortex://new?prompt=…): kept until the page takes them, and
                // announced in case it is already open.
                let open_link = |handle: &tauri::AppHandle, link: chat::links::Link| {
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

            let registry = ToolRegistry::new();
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
            {
                let account = app.state::<account::ActiveAccount>().get();
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(tools::swap_account(&handle, account));
            }

            let settings = load_device_settings(app.handle()).unwrap_or_else(|e| {
                warn!("could not load settings, using defaults: {e}");
                settings::DeviceSettings::default()
            });

            app.handle()
                .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))?;
            let autostart_manager = app.autolaunch();
            if settings.launch_at_startup {
                let _ = autostart_manager.enable();
            } else {
                let _ = autostart_manager.disable();
            }

            if crate::HOSTED_WEB_APP || crate::e2e_testing_enabled() {
                app.add_capability(main_window::web_app_capability(crate::LUMO_URL)?)?;
            }
            main_window::build(app)?;

            let app_handle = app.handle().clone();
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |_app, shortcut, event| {
                        let dictation = crate::dictation::is_shortcut(&app_handle, shortcut);
                        match event.state() {
                            ShortcutState::Pressed if dictation => {
                                crate::dictation::pressed(&app_handle)
                            }
                            ShortcutState::Released if dictation => {
                                crate::dictation::released(&app_handle)
                            }
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

            #[cfg(target_os = "macos")]
            {
                crate::unhandled_keys::install();
                app.set_activation_policy(tauri::ActivationPolicy::Regular);
            }

            if settings.search_shortcut_enabled {
                let shortcut = settings
                    .search_shortcut
                    .parse::<Shortcut>()
                    .unwrap_or_else(|_| Shortcut::new(Some(Modifiers::CONTROL), Code::KeyL));
                app.handle().global_shortcut().register(shortcut)?;
            }
            crate::dictation::startup(app.handle());
            chat::sync::start(app.handle().clone());
            chat::share::start(app.handle());
            chat::telegram::start(app.handle());
            chat::slack::start(app.handle());
            chat::discord::start(app.handle());
            tray::create(app.handle())?;
            updater::startup(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
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
        });
}
