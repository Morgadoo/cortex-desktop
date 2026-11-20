//! Trials: a throwaway copy of the data folder, and a host whose turns run against it, so a
//! change (or an eval case) is tried without touching the live data.

use super::*;

/// What a trial copy of the data folder leaves out: the large and the private (conversations, the
/// code sandboxes, backups, the activity log), and what a trial must not share (the trace database,
/// earlier versions, caches). Everything that shapes an answer comes along: memories, pages,
/// instructions, skills, tools, agents, providers and the eval suites.
pub(super) const NOT_COPIED: &[&str] = &[
    "cache",
    "chat/conversations",
    "chat/sandbox",
    "chat/tool-outputs",
    "chat/backups",
    "chat/previews",
    "chat/favicons",
    "chat/audit",
    "chat/index",
    "chat/workbench",
    "chat/agent-envs",
    "chat/canvases",
    "chat/versions",
    "chat/traces.db",
    "chat/traces.db-wal",
    "chat/traces.db-shm",
    "chat/trash",
    "chat/quarantine",
    "chat/libraries",
    "chat/shares",
];

/// The data folder's top-level folders a trial copies: what shapes an answer (memories, pages,
/// instructions, tools, agents and providers live in `chat`), the skills, and the suites. The
/// top-level settings files come too.
pub(super) const COPIED_FOLDERS: &[&str] = &["chat", "skills", "evals"];

/// A throwaway copy of the data folder in which a change is tried before the user is asked. The
/// live data is never touched. The copy is removed when this is dropped, also when the turn is
/// stopped; one left by a crash or a quit mid-trial is swept at the next start
/// ([`sweep_trials`]).
pub struct TrialFolder {
    pub dir: PathBuf,
}

impl Drop for TrialFolder {
    fn drop(&mut self) {
        // Its trace database and signing key are let go first, so the folder can be removed.
        crate::chat::traces::forget(&self.dir);
        crate::chat::script_tools::forget_signing_key(&self.dir);
        crate::chat::script_tools::forget_rates(&self.dir);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Removes trial copies a crash or a quit left behind; run at start, before any trial.
pub(crate) fn sweep_trials(config_dir: &Path) {
    let _ = std::fs::remove_dir_all(config_dir.join("cache").join("trials"));
}

pub(in crate::chat) fn trial_folder(config_dir: &Path) -> Result<TrialFolder, ChatError> {
    fn copy(from: &Path, to: &Path, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)?.flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap_or(&path);
            if NOT_COPIED.iter().any(|skip| relative == Path::new(skip)) {
                continue;
            }
            // A top-level folder that is a link (a synced `chat`, say) is followed; inside it,
            // links are left out.
            let kind = if from == root {
                // A link that leads nowhere (an unmounted drive) is skipped, not an error.
                match std::fs::metadata(&path) {
                    // Not one that leads back up to the data folder or above: that would copy
                    // the copy into itself.
                    Ok(_)
                        if std::fs::canonicalize(&path).is_ok_and(|target| {
                            std::fs::canonicalize(root).is_ok_and(|r| r.starts_with(&target))
                        }) =>
                    {
                        continue
                    }
                    Ok(meta) => meta.file_type(),
                    Err(_) => continue,
                }
            } else {
                entry.file_type()?
            };
            // At the top, only the folders that shape an answer: anything else there (on macOS
            // the local models and caches share this folder) stays out of the copy.
            if kind.is_dir()
                && from == root
                && !COPIED_FOLDERS.iter().any(|f| relative == Path::new(f))
            {
                continue;
            }
            if kind.is_dir() {
                copy(&path, &to.join(entry.file_name()), root)?;
            } else if kind.is_file() {
                std::fs::copy(&path, to.join(entry.file_name()))?;
            }
        }
        Ok(())
    }
    let folder = TrialFolder {
        dir: config_dir
            .join("cache")
            .join("trials")
            .join(uuid::Uuid::new_v4().simple().to_string()),
    };
    copy(config_dir, &folder.dir, config_dir)
        .map_err(|e| ChatError::Invalid(format!("Could not set up the trial: {e}")))?;
    // Tools in the copy are held to the live folder's signatures.
    crate::chat::script_tools::share_signing_key(config_dir, &folder.dir);
    Ok(folder)
}

/// Runs turns on `parent`'s models against another data folder: a trial's copy.
pub struct InFolder {
    parent: Arc<dyn Host>,
    dir: PathBuf,
    state: crate::chat::ChatState,
}

impl InFolder {
    pub fn new(parent: Arc<dyn Host>, dir: PathBuf) -> Self {
        Self {
            parent,
            dir,
            state: crate::chat::ChatState::default(),
        }
    }
}

#[async_trait::async_trait]
impl Host for InFolder {
    fn config_dir(&self) -> Result<PathBuf, crate::chat::error::ChatError> {
        Ok(self.dir.clone())
    }

    fn emit(&self, _event: ChatEvent) {}

    fn chat_state(&self) -> &crate::chat::ChatState {
        &self.state
    }

    fn registry(&self) -> &crate::tools::registry::ToolRegistry {
        self.parent.registry()
    }

    async fn connect_model(
        &self,
        reference: &str,
    ) -> Result<rig_core::DynModel<rig_core::operation::Completion>, crate::chat::error::ChatError>
    {
        self.parent.connect_model(reference).await
    }

    async fn approve(&self, _: &ChatTool, _: &Value, _: Option<&str>) -> ApprovalDecision {
        ApprovalDecision::Deny
    }

    fn notify(&self, _title: &str, _body: &str) {}
}
