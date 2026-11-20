//! Folder triggers: the files that arrived since a task last looked, once they stopped changing.

use super::*;

/// How long a new file must stay unchanged before it counts (it may still be copying).
pub(super) const SETTLE: Duration = Duration::from_secs(5);

/// The visible files directly in `folder`, with whether each has settled.
pub(super) fn folder_files(
    folder: &Path,
    now: std::time::SystemTime,
) -> Result<Vec<(String, bool)>, String> {
    let entries = std::fs::read_dir(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else { continue };
        // Hidden files and partial downloads are not new documents.
        if !meta.is_file()
            || name.starts_with('.')
            || name.ends_with(".part")
            || name.ends_with(".crdownload")
            || name.ends_with(".tmp")
        {
            continue;
        }
        let settled = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_none_or(|age| age >= SETTLE);
        out.push((name, settled));
    }
    out.sort();
    Ok(out)
}

/// The folder tasks whose folders have new, settled files: each with the prompt naming them.
pub(in crate::chat::automation) fn take_new_files(
    config_dir: &Path,
    now: std::time::SystemTime,
) -> Vec<Task> {
    let mut tasks = load(config_dir);
    let mut started = Vec::new();
    let mut changed = false;
    for task in tasks.iter_mut().filter(|t| t.enabled) {
        let Frequency::NewFiles { folder } = &task.frequency else {
            continue;
        };
        let Ok(files) = folder_files(Path::new(folder), now) else {
            continue;
        };
        let Some(seen) = task.seen.as_mut() else {
            // First look: what is there now is the baseline.
            task.seen = Some(files.into_iter().map(|(n, _)| n).collect());
            changed = true;
            continue;
        };
        let fresh: Vec<String> = files
            .iter()
            .filter(|(n, settled)| *settled && !seen.contains(n))
            .map(|(n, _)| n.clone())
            .collect();
        // Files that went away may come back as new ones later.
        let before = seen.len();
        seen.retain(|n| files.iter().any(|(f, _)| f == n));
        changed |= seen.len() != before;
        if fresh.is_empty() {
            continue;
        }
        seen.extend(fresh.iter().cloned());
        changed = true;
        task.last_run = Some(Utc::now());
        task.last_status = Some(TaskStatus::Running);
        let mut run = task.clone();
        let list: Vec<String> = fresh
            .iter()
            .map(|n| format!("- {}", Path::new(folder).join(n).display()))
            .collect();
        run.prompt = format!(
            "{}\n\nNew files in `{folder}`:\n{}",
            task.prompt,
            list.join("\n")
        );
        started.push(run);
    }
    if changed {
        if let Err(e) = store(config_dir, &tasks) {
            warn!("could not save task state: {e}");
        }
    }
    started
}
