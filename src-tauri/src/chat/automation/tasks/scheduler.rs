//! The app's background loop: due tasks, mirrored libraries and the daily maintenance jobs.

use super::*;

/// Starts the scheduler loop.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let the window and providers settle before the first check.
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut ticks: u64 = 0;
        loop {
            // Mirrored library folders: at start, then every 10 minutes.
            if ticks.is_multiple_of(20) {
                if let Ok(config_dir) = crate::settings::get_config_dir(&app) {
                    let dir = config_dir.clone();
                    let _ =
                        tokio::task::spawn_blocking(move || crate::chat::libraries::sync_all(&dir))
                            .await;
                    // Connector documents: the connectors answer slower, so after the folders.
                    let connectors = crate::chat::tools::registry_connectors(&app);
                    for library in crate::chat::libraries::list(&config_dir)
                        .into_iter()
                        .filter(|l| !l.connectors.is_empty())
                    {
                        match crate::chat::libraries::sync_connectors(
                            &config_dir,
                            &library.id,
                            &connectors,
                        )
                        .await
                        {
                            Ok(r) if r.added + r.updated + r.removed > 0 => log::info!(
                                "synced connectors of library {}: +{} ~{} -{}",
                                library.name,
                                r.added,
                                r.updated,
                                r.removed
                            ),
                            Ok(_) => {}
                            Err(e) => log::warn!(
                                "could not sync connectors of library {}: {e}",
                                library.name
                            ),
                        }
                    }
                }
            }
            ticks += 1;
            if let Ok(config_dir) = crate::settings::get_config_dir(&app) {
                for task in take_due(&config_dir, Utc::now()) {
                    info!("running scheduled task {}", task.title);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::chat::engine::run_task(app, task).await
                    });
                }
                // The kernel's maintenance jobs: lifecycle and regression checks daily, evals
                // nightly when on, the review weekly when on (`maintenance`, tested on its own).
                let due = maintenance(&config_dir, Utc::now()).await;
                let (evals_app, review_app) = (app.clone(), app.clone());
                dispatch(
                    due,
                    move |model| {
                        let host: std::sync::Arc<dyn crate::chat::host::Host> =
                            std::sync::Arc::new(crate::chat::host::TauriHost::new(evals_app));
                        tauri::async_runtime::spawn(async move {
                            crate::chat::evals::nightly_and_notify(&host, &model).await
                        });
                    },
                    move |task| {
                        tauri::async_runtime::spawn(async move {
                            crate::chat::engine::run_task(review_app, task).await
                        });
                    },
                );
                for task in take_new_files(&config_dir, std::time::SystemTime::now()) {
                    info!("new files started task {}", task.title);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::chat::engine::run_task(app, task).await
                    });
                }
                // Feeds: every 10 minutes (the first tick is right after start).
                if ticks % 20 == 1 {
                    for task in take_new_feed_items(&config_dir).await {
                        info!("new feed items started task {}", task.title);
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            crate::chat::engine::run_task(app, task).await
                        });
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

/// Runs a task now, outside its schedule.
pub fn run_now(app: &AppHandle, id: &str) -> Result<(), String> {
    let config_dir = crate::settings::get_config_dir(app)?;
    let task = load(&config_dir)
        .into_iter()
        .find(|t| t.id == id)
        .ok_or("No such task.")?;
    record(
        &config_dir,
        id,
        task.last_conversation.as_deref().unwrap_or_default(),
        TaskStatus::Running,
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move { crate::chat::engine::run_task(app, task).await });
    Ok(())
}

/// What one tick of the scheduler's maintenance found due, and did where it needs no running app.
#[derive(Debug, Default)]
pub struct Maintenance {
    /// The daily lifecycle pass, when it ran.
    pub lifecycle: Option<crate::chat::lifecycle::Report>,
    /// The daily regression checks, run with the lifecycle pass.
    pub checked: Option<crate::chat::script_tools::Checked>,
    /// The model to run the user's eval suites on, when the nightly run is due.
    pub evals_on: Option<String>,
    /// The weekly review to run, when due.
    pub review: Option<Task>,
}

/// Starts what [`maintenance`] found due that needs the app: the eval suites on their model, and
/// the review as a task. What already ran is logged.
pub(in crate::chat::automation) fn dispatch(
    due: Maintenance,
    start_evals: impl FnOnce(String),
    start_review: impl FnOnce(Task),
) {
    if due.lifecycle.as_ref().is_some_and(|r| !r.is_empty()) {
        info!("lifecycle: {:?}", due.lifecycle);
    }
    if due
        .checked
        .as_ref()
        .is_some_and(|c| c != &Default::default())
    {
        info!("regression checks: {:?}", due.checked);
    }
    if let Some(model) = due.evals_on {
        start_evals(model);
    }
    if let Some(task) = due.review {
        info!("running the weekly knowledge review");
        start_review(task);
    }
}

/// The kernel's scheduled maintenance, as of `now`: the lifecycle pass and every written tool's
/// regression checks once a day; the eval suites nightly and the knowledge review (with the
/// week's failures) weekly, when turned on. The loop runs what this returns that needs the app.
pub(in crate::chat::automation) async fn maintenance(
    config_dir: &Path,
    now: DateTime<Utc>,
) -> Maintenance {
    let mut due = Maintenance::default();
    if let Some(report) = crate::chat::lifecycle::daily(config_dir, now).await {
        due.lifecycle = Some(report);
        due.checked = Some(crate::chat::script_tools::check_all(config_dir).await);
    }
    if crate::chat::learning::take_evals(config_dir, now) {
        due.evals_on = crate::chat::providers::load(config_dir).default_model;
    }
    if crate::chat::learning::take_review(config_dir, now) {
        due.review = Some(crate::chat::learning::review_task(config_dir));
    }
    due
}
