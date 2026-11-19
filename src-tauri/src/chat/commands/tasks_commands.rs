//! Scheduled tasks and the calendar of their runs.

use super::*;

#[tauri::command]
pub fn chat_list_tasks(app: AppHandle) -> Result<Vec<TaskView>, String> {
    Ok(task_views(crate::chat::tasks::load(&config_dir(&app)?)))
}

#[tauri::command]
pub fn chat_save_task(
    app: AppHandle,
    task: crate::chat::tasks::Task,
) -> Result<Vec<TaskView>, String> {
    crate::chat::tasks::save(&config_dir(&app)?, task).map(task_views)
}

#[tauri::command]
pub fn chat_delete_task(app: AppHandle, id: String) -> Result<Vec<TaskView>, String> {
    crate::chat::tasks::delete(&config_dir(&app)?, &id).map(task_views)
}

#[tauri::command]
pub fn chat_run_task(app: AppHandle, id: String) -> Result<(), String> {
    crate::chat::tasks::run_now(&app, &id)
}

/// Planned task runs between two times, for the Scheduled page's calendar.
#[tauri::command]
pub fn chat_task_calendar(
    app: AppHandle,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<crate::chat::tasks::Occurrence>, String> {
    if to < from || to - from > chrono::Duration::days(62) {
        return Err("Ask for at most two months.".into());
    }
    Ok(crate::chat::tasks::occurrences(
        &crate::chat::tasks::load(&config_dir(&app)?),
        from,
        to,
    ))
}

/// Picks a calendar (.ics) or contacts (.vcf) file; returns its path.
#[tauri::command]
pub async fn chat_pick_calendar_file(
    app: AppHandle,
    contacts: bool,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file();
    let dialog = if contacts {
        dialog
            .set_title("Choose a contacts file")
            .add_filter("Contacts", &["vcf", "vcard"])
    } else {
        dialog
            .set_title("Choose a calendar file")
            .add_filter("Calendar", &["ics", "ical"])
    };
    dialog.pick_file(move |path| {
        let _ = tx.send(path);
    });
    Ok(rx
        .await
        .map_err(|e| e.to_string())?
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}

/// The calendars and contact files the assistant can read.
#[tauri::command]
pub fn chat_calendar_sources(app: AppHandle) -> Result<crate::chat::calendar::Settings, String> {
    Ok(crate::chat::calendar::load(&config_dir(&app)?))
}

/// Saves the calendars and contact files, after checking each one reads.
#[tauri::command]
pub async fn chat_set_calendar_sources(
    app: AppHandle,
    settings: crate::chat::calendar::Settings,
) -> Result<Vec<String>, String> {
    let mut report = Vec::new();
    for cal in &settings.calendars {
        let text = crate::chat::calendar::fetch(&cal.location)
            .await
            .map_err(|e| format!("{}: {e}", cal.name))?;
        let events = crate::chat::calendar::parse_ics(&text);
        if events.is_empty() && !text.contains("BEGIN:VCALENDAR") {
            return Err(format!(
                "{} is not a calendar (no ICS data there).",
                cal.name
            ));
        }
        report.push(format!("{}: {} events", cal.name, events.len()));
    }
    for book in &settings.contacts {
        let text = crate::chat::calendar::fetch(&book.location)
            .await
            .map_err(|e| format!("{}: {e}", book.name))?;
        let contacts = crate::chat::calendar::parse_vcards(&text);
        if contacts.is_empty() {
            return Err(format!(
                "{} has no contacts (a .vcf file is needed).",
                book.name
            ));
        }
        report.push(format!("{}: {} contacts", book.name, contacts.len()));
    }
    crate::chat::calendar::save(&config_dir(&app)?, &settings)?;
    Ok(report)
}

/// Adds a comment to a scheduled task, or removes one (`remove`, its index).
#[tauri::command]
pub fn chat_task_comment(
    app: AppHandle,
    id: String,
    text: Option<String>,
    remove: Option<usize>,
) -> Result<Vec<crate::chat::tasks::Task>, String> {
    crate::chat::tasks::comment(&config_dir(&app)?, &id, text.as_deref(), remove)
}
