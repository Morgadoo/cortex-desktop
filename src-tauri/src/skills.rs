//! Disposable plaintext runtime copies of encrypted skills. Never read back into the skill store.
//! Execution delegates to the existing shell permission gate using an argument vector.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{State, Webview};
use uuid::Uuid;

use super::account::{AccountId, ActiveAccount};

const MAX_FILES: usize = 201; // Up to 200 assets plus the root SKILL.md.
const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 30 * 1024 * 1024;

#[derive(Deserialize)]
pub struct CacheFile {
    path: String,
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    skill_id: String,
    revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetainedSkill {
    skill_id: String,
}

struct Bundle {
    revision: String,
    path: PathBuf,
    cache_root: PathBuf,
}

impl Drop for Bundle {
    fn drop(&mut self) {
        // The cache and in-flight executions share ownership. Replaced/pruned bundles are
        // removed only after the last execution (including approval waits) releases them.
        if remove_bundle(&self.cache_root, &self.path).is_err() {
            log::warn!("Could not remove retired skill bundle");
        }
    }
}

struct Cache {
    root: PathBuf,
    account: Option<AccountId>,
    bundles: HashMap<String, Arc<Bundle>>,
    available: bool,
}

pub struct SkillCache(Mutex<Cache>);

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn private_directory(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| "Could not create skill runtime directory")?;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Could not inspect skill runtime directory")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Skill runtime directory must not be a link".into());
    }
    Ok(())
}

fn remove(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("Could not inspect cached skill".into()),
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|_| "Could not remove cached skill".into())
}

fn remove_bundle(root: &Path, path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid cached skill path")?;
    if parent.parent() != Some(root) {
        return Err("Cached skill is outside the runtime directory".into());
    }
    for directory in [root, parent] {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("Cached skill directory must not be a link".into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("Could not inspect cached skill directory".into()),
            _ => {}
        }
    }
    remove(path)
}

/// Reject cross-platform aliases as well as traversal. A bundle imported on macOS must remain
/// safe on Windows; case collisions and DOS device names must not select a different file.
fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 1024 {
        return Err("Invalid skill file path".into());
    }
    for part in path.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
            || device
        {
            return Err("Invalid skill file path".into());
        }
    }
    Ok(())
}

fn decode_files(files: Vec<CacheFile>) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if files.is_empty() || files.len() > MAX_FILES {
        return Err("Invalid skill file count".into());
    }
    let mut decoded = BTreeMap::new();
    let mut names = HashSet::new();
    let mut total = 0;
    for file in files {
        validate_path(&file.path)?;
        if !names.insert(file.path.to_lowercase()) {
            return Err("Duplicate skill file path".into());
        }
        if file.data.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
            return Err("Skill file exceeds runtime size limit".into());
        }
        let data = STANDARD
            .decode(&file.data)
            .map_err(|_| "Invalid skill file data")?;
        total += data.len();
        if data.len() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err("Skill bundle exceeds runtime size limit".into());
        }
        decoded.insert(file.path, data);
    }
    // Reject both file/directory aliases (a and a/b) and differently-cased directories.
    let mut directories = HashMap::new();
    for path in decoded.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(dir) = parent.filter(|dir| !dir.as_os_str().is_empty()) {
            let text = dir.to_str().ok_or("Invalid skill directory")?;
            let folded = text.to_lowercase();
            if names.contains(&folded)
                || directories
                    .insert(folded, text)
                    .is_some_and(|previous| previous != text)
            {
                return Err("Conflicting skill paths".into());
            }
            parent = dir.parent();
        }
    }
    Ok(decoded)
}

impl SkillCache {
    fn script_input(
        &self,
        active: &ActiveAccount,
        account: &AccountId,
        request: &ScriptRequest,
    ) -> Result<(serde_json::Value, Arc<Bundle>), String> {
        let cache = self.0.lock().map_err(|_| "Skill cache unavailable")?;
        Self::authorize(&cache, active, account)?;
        let bundle = cache
            .bundles
            .get(&request.skill_id)
            .ok_or("Skill bundle is not synced")?;
        if bundle.revision != request.revision {
            return Err("Skill revision changed; retry execution".into());
        }
        validate_path(&request.script_path)?;
        if !request.skill_path.is_empty() {
            validate_path(&request.skill_path)?;
        }
        if request.args.len() > 256
            || request
                .args
                .iter()
                .any(|arg| arg.len() > 128 * 1024 || arg.contains('\0'))
        {
            return Err("Invalid script arguments".into());
        }
        let script = bundle.path.join(&request.script_path);
        let root = bundle
            .path
            .canonicalize()
            .map_err(|_| "Skill bundle is unavailable")?;
        let resolved = script.canonicalize().map_err(|_| "Script is unavailable")?;
        if !resolved.starts_with(&root) || !resolved.is_file() {
            return Err("Invalid bundled script".into());
        }
        let program = match script.extension().and_then(|ext| ext.to_str()) {
            Some("py") => {
                if cfg!(windows) {
                    "python"
                } else {
                    "python3"
                }
            }
            Some("js" | "mjs" | "cjs") => "node",
            Some("sh") => "sh",
            Some("bash") => "bash",
            Some("zsh") => "zsh",
            Some("rb") => "ruby",
            Some("pl") => "perl",
            Some("ps1") => "pwsh",
            _ => return Err("Unsupported script interpreter".into()),
        };
        let skill_root = root.join(&request.skill_path);
        let replace = |value: &str| {
            value
                .replace("${CLAUDE_PLUGIN_ROOT}", &root.to_string_lossy())
                .replace("${CLAUDE_SKILL_DIR}", &skill_root.to_string_lossy())
        };
        let mut args = vec![resolved.to_string_lossy().into_owned()];
        args.extend(request.args.iter().map(|arg| replace(arg)));
        Ok((
            serde_json::json!({ "script": { "program": program, "args": args } }),
            Arc::clone(bundle),
        ))
    }

    pub fn new(root: PathBuf, account: Option<AccountId>) -> Self {
        // Single-instance Desktop owns this directory. Discard plaintext left by a crashed run.
        let available = remove(&root).is_ok();
        Self(Mutex::new(Cache {
            root,
            account,
            bundles: HashMap::new(),
            available,
        }))
    }

    pub fn activate(&self, account: Option<AccountId>) {
        let mut cache = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.account != account {
            // In-flight materializations hold this same lock, so sign-out cannot be followed by
            // an old request recreating its plaintext files.
            cache.available = remove(&cache.root).is_ok();
            if !cache.available {
                log::warn!("Could not clear skill runtime cache");
            }
            cache.bundles.clear();
            cache.account = account;
        }
    }

    fn authorize(cache: &Cache, active: &ActiveAccount, account: &AccountId) -> Result<(), String> {
        if !cache.available {
            return Err("Skill runtime cache is unavailable".into());
        }
        if cache.account.as_ref() != Some(account) || active.get().as_ref() != Some(account) {
            return Err("Skill cache account does not match the active account".into());
        }
        Ok(())
    }

    fn materialize(
        &self,
        active: &ActiveAccount,
        account: &AccountId,
        entry: CacheEntry,
        files: Vec<CacheFile>,
    ) -> Result<(), String> {
        if entry.skill_id.is_empty() || entry.skill_id.len() > 128 || entry.revision.len() > 128 {
            return Err("Invalid skill cache entry".into());
        }
        let mut cache = self.0.lock().map_err(|_| "Skill cache unavailable")?;
        Self::authorize(&cache, active, account)?;
        // Clients supplies a content-derived revision. Local files are not a sync input.
        if cache
            .bundles
            .get(&entry.skill_id)
            .is_some_and(|bundle| bundle.revision == entry.revision)
        {
            return Ok(());
        }
        let files = decode_files(files)?;
        private_directory(&cache.root)?;
        let account_dir = cache.root.join(digest(account.as_str().as_bytes()));
        private_directory(&account_dir)?;
        let path = account_dir.join(format!(
            "{}-{}",
            digest(entry.skill_id.as_bytes()),
            Uuid::new_v4()
        ));
        let staging = account_dir.join(format!("staging-{}", Uuid::new_v4()));
        private_directory(&staging)?;
        let write: Result<(), String> = (|| {
            for (name, data) in &files {
                let file = staging.join(name);
                private_directory(file.parent().ok_or("Invalid skill file path")?)?;
                // create_new also catches filesystem aliases (e.g. Unicode normalization)
                // that cannot be detected portably by comparing incoming path strings.
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(file)
                    .and_then(|mut file| file.write_all(data))
                    .map_err(|_| "Could not write cached skill")?;
            }
            fs::rename(&staging, &path).map_err(|_| "Could not publish cached skill".to_string())
        })();
        if let Err(error) = write {
            let _ = remove(&staging);
            return Err(error);
        }
        let cache_root = cache.root.clone();
        cache.bundles.insert(
            entry.skill_id,
            Arc::new(Bundle {
                revision: entry.revision,
                path,
                cache_root,
            }),
        );
        Ok(())
    }

    fn retain(
        &self,
        active: &ActiveAccount,
        account: &AccountId,
        entries: Vec<RetainedSkill>,
    ) -> Result<(), String> {
        let mut cache = self.0.lock().map_err(|_| "Skill cache unavailable")?;
        Self::authorize(&cache, active, account)?;
        let live: HashSet<_> = entries.into_iter().map(|entry| entry.skill_id).collect();
        let stale: Vec<_> = cache
            .bundles
            .iter()
            .filter(|(id, _)| !live.contains(*id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            cache.bundles.remove(&id);
        }
        Ok(())
    }
}

impl Drop for SkillCache {
    fn drop(&mut self) {
        if let Ok(cache) = self.0.get_mut() {
            let _ = remove(&cache.root);
        }
    }
}

fn require_main(label: &str) -> Result<(), String> {
    crate::main_window::require(label, "Skill cache requires the main webview")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRequest {
    skill_id: String,
    revision: String,
    skill_path: String,
    script_path: String,
    args: Vec<String>,
}

#[tauri::command]
pub async fn skill_execute_script(
    request: ScriptRequest,
    user_id: String,
    webview: Webview,
    active: State<'_, ActiveAccount>,
    cache: State<'_, SkillCache>,
    registry: State<'_, super::tools::registry::ToolRegistry>,
) -> Result<serde_json::Value, String> {
    require_main(webview.label())?;
    let (input, _bundle_lease) =
        cache.script_input(&active, &AccountId::parse(&user_id)?, &request)?;
    // Normal shell readiness, approval, disabled/read-only policy, and account result scope apply.
    registry
        .dispatch("bash", "run", input)
        .await
        .map_err(|error| error.to_ipc_string())
}

#[tauri::command]
pub async fn skill_sync_bundle(
    user_id: String,
    skill_id: String,
    revision: String,
    files: Vec<CacheFile>,
    webview: Webview,
    active: State<'_, ActiveAccount>,
    cache: State<'_, SkillCache>,
) -> Result<(), String> {
    require_main(webview.label())?;
    cache.materialize(
        &active,
        &AccountId::parse(&user_id)?,
        CacheEntry { skill_id, revision },
        files,
    )
}

#[tauri::command]
pub async fn skill_cache_retain(
    user_id: String,
    skills: Vec<RetainedSkill>,
    webview: Webview,
    active: State<'_, ActiveAccount>,
    cache: State<'_, SkillCache>,
) -> Result<(), String> {
    require_main(webview.label())?;
    cache.retain(&active, &AccountId::parse(&user_id)?, skills)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::tools::{
        connector_state::ToolPolicy, connectors::BashConnector, registry::ToolRegistry,
    };

    #[tokio::test]
    async fn synced_script_executes_literal_arguments_and_respects_lifecycle_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let account = AccountId::parse("test-account").unwrap();
        let active = ActiveAccount::default();
        active.set(Some(account.clone()));
        let cache = SkillCache::new(dir.path().join("cache"), Some(account.clone()));
        let file = |path: &str, data: &[u8]| CacheFile {
            path: path.into(),
            data: STANDARD.encode(data),
        };
        let entry = || CacheEntry {
            skill_id: "bundle".into(),
            revision: "1".into(),
        };
        cache
            .materialize(
                &active,
                &account,
                entry(),
                vec![file("scripts/run.sh", b"printf '%s' \"$1\"")],
            )
            .unwrap();
        let request = ScriptRequest {
            skill_id: "bundle".into(),
            revision: "1".into(),
            skill_path: "".into(),
            script_path: "scripts/run.sh".into(),
            args: vec!["hello ' $(touch SHOULD_NOT_EXIST) ; &".into()],
        };
        let (input, bundle_lease) = cache.script_input(&active, &account, &request).unwrap();
        let script = PathBuf::from(input["script"]["args"][0].as_str().unwrap());
        let registry = ToolRegistry::new();
        registry.register(BashConnector::new(true, vec![dir.path().to_path_buf()]));
        // An absent approval gate refuses execution, as for all ordinary shell calls.
        registry.set_tool_policy("bash", "run", ToolPolicy::Ask);
        assert!(registry
            .dispatch("bash", "run", input.clone())
            .await
            .is_err());
        registry.set_tool_policy("bash", "run", ToolPolicy::AutoApprove);
        let result = registry.dispatch("bash", "run", input).await.unwrap();
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["stdout"], request.args[0]);
        drop(bundle_lease);
        assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
        assert!(cache
            .materialize(
                &active,
                &account,
                CacheEntry {
                    skill_id: "bundle".into(),
                    revision: "2".into()
                },
                vec![file("../escape", b"x")]
            )
            .is_err());
        cache.retain(&active, &account, vec![]).unwrap();
        assert!(!script.exists());
        assert!(cache.script_input(&active, &account, &request).is_err());
        cache.activate(None);
        assert!(cache
            .materialize(&active, &account, entry(), vec![file("x", b"x")])
            .is_err());
    }
}
