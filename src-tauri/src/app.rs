use log::{error, info, warn};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::settings::{self, load_device_settings};
use crate::tools::{self, registry::ToolRegistry};
use crate::{
    account, browser_login, chat, downloads, main_window, notifications, pdf_export, quick_ask,
    reporting, reset, schedules, skills, theme, tray, updater, webview,
};

pub fn run() {
    // The guard must outlive the event loop to prevent premature client shutdown.
    let _sentry = reporting::init();
    tauri::Builder::default()
        .invoke_handler(|invoke| {
            let app = invoke.message.webview().app_handle().clone();
            if updater::is_required(&app) {
                invoke.resolver.reject("Update Lumo to continue");
                return true;
            }
            let commands: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                quick_ask::window::inject_search,
                quick_ask::window::hide_search,
                quick_ask::take_quick_ask,
                settings::commands::get_app_settings,
                settings::commands::save_app_settings,
                settings::window::close_settings_window,
                settings::window::open_settings_window,
                settings::window::take_pending_settings_tab,
                reset::reset_local_configuration,
                theme::report_web_theme,
                theme::get_web_theme,
                browser_login::start_browser_login,
                crate::feature_flags::get_desktop_feature_flags,
                crate::feature_flags::set_desktop_feature_flags,
                account::is_account_signed_in,
                account::set_active_account,
                account::clear_active_account,
                skills::skill_sync_bundle,
                skills::skill_cache_retain,
                skills::skill_execute_script,
                notifications::desktop_notification_permission,
                notifications::desktop_notification_permission_state,
                notifications::desktop_notification_show,
                notifications::desktop_notification_open_settings,
                downloads::reveal_download,
                pdf_export::export_pdf,
                schedules::bridge::schedule_connect,
                schedules::bridge::schedule_replace,
                schedules::bridge::schedule_run_now,
                schedules::bridge::schedule_pending,
                schedules::bridge::schedule_unsettled,
                schedules::bridge::schedule_report_status,
                tools::commands::get_tool_registry,
                tools::commands::get_builtin_tools,
                tools::commands::is_secure_storage_available,
                tools::commands::set_connector_enabled,
                tools::commands::set_tool_policy,
                tools::commands::respond_tool_approval,
                tools::commands::always_allow_tool,
                tools::commands::invoke_tool,
                tools::commands::register_schedule_execution_context,
                tools::commands::connect_connector,
                tools::commands::cancel_connector_connection,
                tools::commands::disconnect_connector,
                tools::commands::save_mcp_server,
                tools::commands::list_mcp_servers,
                tools::commands::remove_mcp_server,
                tools::commands::pick_directory,
                chat::commands::chat_list_providers,
                chat::commands::chat_save_provider,
                chat::commands::chat_list_models,
                chat::commands::chat_set_default_model,
                chat::commands::chat_test_model,
                chat::commands::chat_list_conversations,
                chat::commands::chat_get_conversation,
                chat::commands::chat_send,
                chat::commands::chat_cancel,
                chat::commands::chat_answer,
                chat::commands::chat_update_conversation,
                chat::commands::chat_delete_conversation,
                chat::commands::chat_list_projects,
                chat::commands::chat_save_project,
                chat::commands::chat_delete_project,
                chat::commands::chat_get_instructions,
                chat::commands::chat_save_instructions,
                chat::commands::chat_preview_instructions,
                chat::commands::chat_export_conversation,
                chat::commands::chat_get_memories,
                chat::commands::chat_add_memory,
                chat::commands::chat_update_memory,
                chat::commands::chat_delete_memory,
                chat::commands::chat_set_memory_enabled,
                chat::commands::chat_list_skills,
                chat::commands::chat_save_skill,
                chat::commands::chat_delete_skill,
                chat::commands::chat_set_skill_enabled,
                chat::commands::chat_import_skill,
                chat::commands::chat_get_canvas,
                chat::commands::chat_save_canvas,
                chat::commands::chat_export_canvas,
                chat::commands::chat_list_agents,
                chat::commands::chat_agent_templates,
                chat::commands::chat_save_agent,
                chat::commands::chat_delete_agent,
                chat::commands::chat_list_tasks,
                chat::commands::chat_save_task,
                chat::commands::chat_delete_task,
                chat::commands::chat_run_task,
                chat::commands::chat_list_libraries,
                chat::commands::chat_create_library,
                chat::commands::chat_delete_library,
                chat::commands::chat_add_documents,
                chat::commands::chat_remove_document,
                chat::commands::chat_search_libraries,
                chat::commands::chat_get_privacy,
                chat::commands::chat_set_privacy,
                chat::commands::chat_project_library,
                chat::commands::chat_export_all,
                chat::commands::chat_list_apps,
                chat::commands::chat_keep_conversation,
                chat::commands::chat_media_models,
                chat::commands::chat_code_changes,
                chat::commands::chat_workflows,
                chat::commands::chat_save_workflow,
                chat::commands::chat_delete_workflow,
                chat::commands::chat_code_runs_allowed,
                chat::commands::chat_set_code_runs_allowed,
                chat::commands::chat_speak,
                chat::commands::chat_steer,
                chat::commands::chat_audio_settings,
                chat::commands::chat_save_audio_settings,
                chat::commands::chat_audio_voices,
                chat::commands::chat_search,
                chat::commands::chat_trash,
                chat::commands::chat_restore_conversation,
                chat::commands::chat_delete_forever,
                chat::commands::chat_storage_health,
                chat::commands::chat_recover_conversation,
                chat::commands::chat_context_breakdown,
                chat::commands::chat_fork_conversation,
                chat::commands::chat_agent_notebook,
                chat::commands::chat_save_agent_notebook,
                chat::commands::chat_agent_spend,
                chat::commands::chat_draft_agent,
                chat::commands::chat_usage_month,
                chat::commands::chat_subagent_transcript,
                chat::commands::chat_code_files,
                chat::commands::chat_code_read_file,
                chat::commands::chat_favicon,
                chat::commands::chat_link_preview,
                chat::commands::chat_debug,
            chat::commands::chat_read_document,
            chat::commands::chat_notify,
            chat::commands::chat_rewrite_message,
            chat::commands::chat_take_link,
            chat::commands::chat_bookmark,
            chat::commands::chat_versions,
            chat::commands::chat_document_text,
            chat::commands::chat_api_server_log,
            chat::commands::chat_import_skill_url,
            chat::commands::chat_add_web_page,
            chat::commands::chat_diagnostics_prefs,
            chat::commands::chat_set_diagnostics_prefs,
            chat::commands::chat_export_diagnostics,
            chat::commands::chat_backup_model,
            chat::commands::chat_set_backup_model,
            chat::commands::chat_move_document,
            chat::commands::chat_get_defaults,
            chat::commands::chat_version_texts,
            chat::commands::chat_list_canvases,
            chat::commands::chat_improve_text,
            chat::commands::chat_recent_files,
            chat::commands::chat_remember_files,
            chat::commands::chat_forget_recent_file,
            chat::commands::chat_delete_exchanges,
            chat::commands::chat_import_plugin,
            chat::commands::chat_plugin_marketplace,
            chat::commands::chat_get_hooks,
            chat::commands::chat_save_hooks,
            chat::commands::chat_create_worktree,
            chat::commands::chat_file_memory,
            chat::commands::chat_import_archive,
            chat::commands::chat_open_app_window,
            chat::commands::chat_set_defaults,
                chat::commands::chat_custom_tools,
                chat::commands::chat_save_custom_tool,
                chat::commands::chat_delete_custom_tool,
                chat::commands::chat_test_custom_tool,
                chat::commands::chat_arena_record,
                chat::commands::chat_arena_board,
                chat::commands::chat_arena_reset,
                chat::commands::chat_skill_history,
                chat::commands::chat_restore_skill,
                chat::commands::chat_add_library_folder,
                chat::commands::chat_remove_library_folder,
                chat::commands::chat_sync_library,
                chat::commands::chat_conversation_text,
                chat::commands::chat_rate_reason,
                chat::commands::chat_save_file,
                chat::commands::chat_run_snippet,
                chat::commands::chat_snippet_languages,
                chat::commands::chat_translate,
                chat::commands::chat_computer_settings,
                chat::commands::chat_save_computer_settings,
                chat::commands::chat_openapi_servers,
                chat::commands::chat_add_openapi_server,
                chat::commands::chat_refresh_openapi_server,
                chat::commands::chat_toggle_openapi_server,
                chat::commands::chat_remove_openapi_server,
                chat::commands::chat_task_calendar,
                chat::commands::chat_api_server,
                chat::commands::chat_save_api_server,
                chat::commands::chat_delete_exchange,
                chat::commands::chat_export_agent,
                chat::commands::chat_import_agent,
                chat::commands::chat_export_conversation_json,
                chat::commands::chat_search_settings,
                chat::commands::chat_save_search_settings,
                chat::commands::chat_embedding_settings,
                chat::commands::chat_save_embedding_settings,
                chat::commands::chat_index_libraries,
                chat::commands::chat_set_params,
                chat::commands::chat_follow_ups,
                chat::commands::chat_save_shared_page,
                chat::commands::chat_read_spreadsheet,
                chat::commands::chat_terminal_list,
                chat::commands::chat_terminal_start,
                chat::commands::chat_terminal_stop,
                chat::commands::chat_terminal_clear,
                chat::commands::chat_knowledge_pages,
                chat::commands::chat_save_knowledge_page,
                chat::commands::chat_delete_knowledge_page,
                chat::commands::chat_code_commit,
                chat::commands::chat_code_pull_request,
                chat::commands::chat_select_version,
                chat::commands::chat_import_conversations,
                chat::commands::chat_set_media_models,
                chat::commands::chat_transcribe,
            ];
            commands(invoke)
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
        .register_uri_scheme_protocol("lumoapp", |ctx, request| {
            match crate::settings::get_config_dir(ctx.app_handle()) {
                Ok(dir) => match request.uri().path().strip_prefix("/sandbox/") {
                    Some(rest) => chat::sandbox::serve(&dir, rest),
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
            // First line of every launch: the log appends across sessions, and a bug report is
            // only useful with the version and platform it came from.
            info!(
                "Lumo {} starting on {} {} ({})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                if cfg!(debug_assertions) { "debug" } else { "release" }
            );

            if let Err(message) = reset::apply_pending_reset(app.handle()) {
                error!("local configuration reset failed: {message}");
                let handle = app.handle().clone();
                app.dialog()
                    .message(format!("Could not reset local configuration: {message}\n\nRetry later closes Lumo and retries the reset on the next launch. Cancel reset reopens Lumo without retrying. Changes already made will remain."))
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
            chat::commands::setup(app);
            app.manage(skills::SkillCache::new(app.path().app_cache_dir()?.join("skill-runtime"),
                app.state::<account::ActiveAccount>().get()));
            app.manage(schedules::coordinator::ScheduleCoordinator::start(app.handle()));

            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.manage(chat::links::PendingLink::default());
                // Links into the app (lumo://new?prompt=…): kept until the page takes them, and
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

            match settings::get_config_dir(app.handle()) {
                Ok(config_dir) => tools::auth::set_pending_deletes_dir(config_dir),
                Err(e) => error!("connector secrets will not load: {e}"),
            }
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
                    .with_handler(move |_app, _shortcut, event| {
                        if event.state() == ShortcutState::Pressed {
                            if let Err(e) = quick_ask::window::show(&app_handle) {
                                error!("could not show search window: {e}");
                            }
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
            tray::create(app.handle())?;
            updater::startup(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                if let Err(e) = main_window::show(app) {
                    warn!("failed to show main window on reopen: {e}");
                }
            }

            if matches!(event, tauri::RunEvent::Exit) {
                // Background commands from coding sessions end with the app.
                chat::terminal::stop_all();
                if let Some(cache) = app.try_state::<skills::SkillCache>() {
                    cache.activate(None);
                }
                if let Some(coordinator) =
                    app.try_state::<schedules::coordinator::ScheduleCoordinator>()
                {
                    coordinator.shutdown();
                }
            }
        });
}
