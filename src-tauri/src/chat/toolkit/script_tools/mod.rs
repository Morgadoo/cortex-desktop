//! Tools the assistant writes for itself: a short JavaScript script with a name, a description and
//! an argument schema, run in the WebAssembly [`sandbox`].
//!
//! The lifecycle follows the kernel's rules. `create_tool` saves a *candidate* with no
//! capabilities and runs it once on test arguments; creating and testing is free. A candidate is
//! never offered to the model. `promote_tool` always asks the user, in every mode, showing the
//! script, the description the model will read and the capabilities asked for; once approved the
//! tool is *active*, its capabilities granted, and its script pinned by hash: a changed script is
//! refused until promoted again. Retired tools are *deprecated*: hidden, and restorable.
//!
//! Tools live in `chat/script-tools/<name>.json`, sealed at rest like the rest of the app's data.

pub mod sandbox;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat::store::chat_dir;
use cortex_kernel::tool::ChatTool;
pub use sandbox::{Capabilities, Limits};

/// The connector id script tools are offered under (`script__<name>`).
pub const CONNECTOR: &str = "script";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Candidate,
    Active,
    Deprecated,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Candidate => "candidate",
            Status::Active => "active",
            Status::Deprecated => "deprecated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub script: String,
    /// What the tool asked for when it was written.
    pub requested: Capabilities,
    /// What the user granted when promoting it; nothing for a candidate.
    #[serde(default)]
    pub granted: Capabilities,
    pub status: Status,
    /// The hash of the script the user approved; `None` until promoted.
    #[serde(default)]
    pub approved_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    /// The conversation it was written in.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Its regression checks: arguments and the result they must keep giving. A test run that
    /// worked becomes one when the tool is promoted.
    #[serde(default)]
    pub checks: Vec<Check>,
    /// The latest test run, waiting to become a check.
    #[serde(default)]
    pub pending_check: Option<Check>,
    /// When the granted capabilities lapse; never when absent.
    #[serde(default)]
    pub granted_until: Option<DateTime<Utc>>,
    /// The app's signature over what the user approved, what the tool may reach and what the
    /// monitor and History go by (see `signed_part`), keyed by a secret in the OS keychain: a file
    /// changed outside the app no longer verifies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// When each granted capability lapses (`http:host`, `read:folder`, `write:folder`,
    /// `tool:name`, `cred:host=SECRET`); one without an entry lapses at `granted_until`, or never.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub expires: std::collections::BTreeMap<String, DateTime<Utc>>,
    /// When the monitor last rolled this tool back, so the same week's failures do not roll it
    /// back again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_reverted_at: Option<DateTime<Utc>>,
    /// When the user last approved it: the lifecycle and the monitor judge only calls since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    /// Versions (by approval hash) the user set aside by bringing back an earlier one from
    /// History: the monitor never rolls forward to them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_aside: Vec<String>,
    /// When the user last brought back an earlier version from History: until the version has a
    /// week of its own calls as a baseline, it is not compared week over week, since "last week"
    /// would be the versions the user set aside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<DateTime<Utc>>,
    /// How many times the app has signed this tool: each signed save is numbered above every one
    /// before it, so a later save whose record never reached the keychain (a crash, a failed
    /// write) is still told from an earlier one put back.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seq: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Each capability item with its key: `http:host`, `read:folder`, and so on.
fn keyed(c: &Capabilities) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, items) in [
        ("http", &c.http_hosts),
        ("read", &c.read_prefixes),
        ("write", &c.write_prefixes),
        ("tool", &c.tools),
        ("cred", &c.credentials),
    ] {
        out.extend(items.iter().map(|i| format!("{kind}:{i}")));
    }
    out
}

/// What a tool may reach at `now`: its grants, less each that has lapsed.
pub fn active_grants(tool: &ScriptTool, now: DateTime<Utc>) -> Capabilities {
    let live = |kind: &str, item: &String| {
        tool.expires
            .get(&format!("{kind}:{item}"))
            .copied()
            .or(tool.granted_until)
            .is_none_or(|until| until > now)
    };
    let keep = |kind: &str, items: &[String]| -> Vec<String> {
        items.iter().filter(|i| live(kind, i)).cloned().collect()
    };
    Capabilities {
        http_hosts: keep("http", &tool.granted.http_hosts),
        read_prefixes: keep("read", &tool.granted.read_prefixes),
        write_prefixes: keep("write", &tool.granted.write_prefixes),
        tools: keep("tool", &tool.granted.tools),
        credentials: keep("cred", &tool.granted.credentials),
    }
}

const SIGNING_KEY_NAME: &str = "tool-signing-key";

type Keys = std::sync::RwLock<Vec<(PathBuf, [u8; 32])>>;

fn keys() -> &'static Keys {
    static KEYS: std::sync::OnceLock<Keys> = std::sync::OnceLock::new();
    KEYS.get_or_init(Default::default)
}

/// Signs and verifies the tools in `config_dir` with `key` from now on.
pub fn set_signing_key(config_dir: &Path, key: [u8; 32]) {
    let mut keys = keys().write().unwrap_or_else(|p| p.into_inner());
    keys.retain(|(dir, _)| dir != config_dir);
    keys.push((config_dir.to_path_buf(), key));
}

/// Live keys a trial's copy checks what it copied against; it never signs with them.
fn inherited() -> &'static Keys {
    static INHERITED: std::sync::OnceLock<Keys> = std::sync::OnceLock::new();
    INHERITED.get_or_init(Default::default)
}

/// A trial's copy signs with a key of its own, derived from the live one and the copy's folder,
/// so nothing it signs verifies in the live folder; it checks what it copied against the live key.
fn set_trial_keys(trial: &Path, live: [u8; 32]) {
    let derived = hmac(&live, format!("trial-copy:{}", trial.display()).as_bytes());
    let bytes: Vec<u8> = (0..32)
        .filter_map(|i| u8::from_str_radix(&derived[i * 2..i * 2 + 2], 16).ok())
        .collect();
    if let Ok(own) = <[u8; 32]>::try_from(bytes) {
        set_signing_key(trial, own);
    }
    let mut inherited = inherited().write().unwrap_or_else(|p| p.into_inner());
    inherited.retain(|(dir, _)| dir != trial);
    inherited.push((trial.to_path_buf(), live));
}

/// Gives a trial's copy the live folder's key and its record of each tool's latest signature, so
/// a file changed outside the app is refused there as it is in the app. The copy's record is never
/// kept in the keychain.
pub fn share_signing_key(from: &Path, to: &Path) {
    if let Some(key) = signing_key(from) {
        set_trial_keys(to, key);
    }
    let mut ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
    ledger.trials.retain(|dir| dir != to);
    ledger.trials.push(to.to_path_buf());
    let shared: Vec<(String, Latest)> = ledger
        .latest
        .iter()
        .filter(|((dir, _), _)| dir == from)
        .map(|((_, name), latest)| (name.clone(), latest.clone()))
        .collect();
    for (name, latest) in shared {
        ledger.latest.insert((to.to_path_buf(), name), latest);
    }
}

/// Drops the key for a folder that is going away (a trial's copy).
pub fn forget_signing_key(config_dir: &Path) {
    keys()
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|(dir, _)| dir != config_dir);
    inherited()
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|(dir, _)| dir != config_dir);
    let mut ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
    ledger.latest.retain(|(dir, _), _| dir != config_dir);
    ledger.stores.retain(|(dir, _)| dir != config_dir);
    ledger.trials.retain(|dir| dir != config_dir);
    ledger.unread.retain(|dir| dir != config_dir);
}

/// Each tool's latest signature, per folder. A signature only shows the app wrote a state; this
/// record shows which state is the latest, so an earlier signed state put back over a tool's file
/// does not verify. It is kept in the keychain, outside the data folder, for the folders whose key
/// came from there.
#[derive(Default)]
struct Ledger {
    latest: std::collections::HashMap<(PathBuf, String), Latest>,
    stores: Vec<(PathBuf, crate::tools::auth::Secrets)>,
    /// Trial copies: their record is never kept in the keychain, whatever key they hold.
    trials: Vec<PathBuf>,
    /// Folders whose record could not be read from the keychain this session: no tool there is
    /// taken for one the app never signed.
    unread: Vec<PathBuf>,
}

/// A tool's latest signature and the number of the save that made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Latest {
    sig: String,
    seq: u64,
}

fn ledger() -> &'static std::sync::Mutex<Ledger> {
    static LEDGER: std::sync::OnceLock<std::sync::Mutex<Ledger>> = std::sync::OnceLock::new();
    LEDGER.get_or_init(Default::default)
}

/// The keychain entry holding a folder's record of latest signatures.
fn latest_entry(config_dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
    let short: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("tool-signing-latest-{short}")
}

/// Whether the app ever signed a tool by this name in this folder. When the folder's record could
/// not be read, it may have: so it is taken that it did.
fn ever_signed(config_dir: &Path, name: &str) -> bool {
    let ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
    ledger.unread.iter().any(|dir| dir == config_dir)
        || ledger
            .latest
            .contains_key(&(config_dir.to_path_buf(), name.to_string()))
}

/// The number of the latest save recorded for a tool, 0 when none is.
fn recorded_seq(config_dir: &Path, name: &str) -> u64 {
    ledger()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .latest
        .get(&(config_dir.to_path_buf(), name.to_string()))
        .map_or(0, |l| l.seq)
}

/// Records a tool's latest signature and keeps the folder's record in the keychain.
fn record_latest(config_dir: &Path, name: &str, signature: &str, seq: u64) {
    let store = {
        let mut ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
        ledger.latest.insert(
            (config_dir.to_path_buf(), name.to_string()),
            Latest {
                sig: signature.to_string(),
                seq,
            },
        );
        ledger
            .stores
            .iter()
            .find(|(dir, _)| dir == config_dir)
            .map(|(_, store)| store.clone())
    };
    if let Some(store) = store {
        let config_dir = config_dir.to_path_buf();
        tauri::async_runtime::spawn(async move { keep_latest(&config_dir, &store).await });
    }
}

/// Writes a folder's record to the keychain as it stands when the write runs, one write at a time,
/// so a slower earlier write never leaves an older record behind.
async fn keep_latest(config_dir: &Path, store: &crate::tools::auth::Secrets) {
    static WRITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = WRITING.lock().await;
    let record: std::collections::BTreeMap<String, Latest> = ledger()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .latest
        .iter()
        .filter(|((dir, _), _)| dir == config_dir)
        .map(|((_, name), latest)| (name.clone(), latest.clone()))
        .collect();
    let text = serde_json::to_string(&record).unwrap_or_default();
    if let Err(e) = store.store(&latest_entry(config_dir), &text).await {
        log::warn!("the record of tools' latest signatures was not kept: {e}");
    }
}

/// Loads a folder's record of latest signatures from the keychain. A tool whose file holds a later
/// signed save than the record (one whose record never reached the keychain), or one signed before
/// there was a record, is recorded as it stands. A record that cannot be read is not written over.
async fn load_latest(config_dir: &Path, store: &crate::tools::auth::Secrets) {
    let record: Option<std::collections::BTreeMap<String, Latest>> =
        match store.load(&latest_entry(config_dir)).await {
            Ok(None) => Some(Default::default()),
            Ok(Some(text)) => serde_json::from_str(&text).ok(),
            Err(_) => None,
        };
    let Some(record) = record else {
        log::warn!("the record of tools' latest signatures could not be read; it is left as it is");
        let mut ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
        ledger.unread.retain(|dir| dir != config_dir);
        ledger.unread.push(config_dir.to_path_buf());
        return;
    };
    {
        let mut ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
        for (name, latest) in record {
            ledger
                .latest
                .insert((config_dir.to_path_buf(), name), latest);
        }
        ledger.stores.retain(|(dir, _)| dir != config_dir);
        ledger
            .stores
            .push((config_dir.to_path_buf(), store.clone()));
    }
    for tool in list(config_dir) {
        let behind =
            !ever_signed(config_dir, &tool.name) || tool.seq > recorded_seq(config_dir, &tool.name);
        if behind && signed_by_app(config_dir, &tool) {
            if let Some(sig) = &tool.signature {
                let mut ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
                ledger.latest.insert(
                    (config_dir.to_path_buf(), tool.name.clone()),
                    Latest {
                        sig: sig.clone(),
                        seq: tool.seq,
                    },
                );
            }
        }
    }
    keep_latest(config_dir, store).await;
}

fn signing_key(config_dir: &Path) -> Option<[u8; 32]> {
    keys()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|(dir, _)| dir == config_dir)
        .map(|(_, key)| *key)
}

/// The signing key from the keychain, made the first time it is needed. Without a keychain,
/// tools are not signed and the hash pin alone holds them to what the user approved.
pub async fn ensure_signing_key(config_dir: &Path) {
    if signing_key(config_dir).is_some() {
        return;
    }
    match crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT) {
        Ok(account) => {
            let store = crate::tools::auth::keychain_secrets(account);
            ensure_signing_key_in(config_dir, &store).await
        }
        Err(e) => log::warn!("tools the assistant wrote are not signed: {e}"),
    }
}

/// [`ensure_signing_key`] with `store` for the keychain: reads the key, or makes and keeps one and
/// reads it back to be sure it is there.
pub async fn ensure_signing_key_in(config_dir: &Path, store: &crate::tools::auth::Secrets) {
    let key = async {
        if let Some(hex) = store.load(SIGNING_KEY_NAME).await? {
            return Ok::<_, String>(Some(hex));
        }
        let mut key = [0u8; 32];
        rand::fill(&mut key);
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        store.store(SIGNING_KEY_NAME, &hex).await?;
        store.load(SIGNING_KEY_NAME).await
    }
    .await;
    let parsed = key.ok().flatten().and_then(|hex| {
        let bytes: Option<Vec<u8>> = (0..32)
            .map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok())
            .collect();
        <[u8; 32]>::try_from(bytes?).ok()
    });
    match parsed {
        Some(key) => {
            let trial = ledger()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .trials
                .iter()
                .any(|dir| dir == config_dir);
            if trial {
                set_trial_keys(config_dir, key);
            } else {
                set_signing_key(config_dir, key);
                load_latest(config_dir, store).await;
            }
        }
        None => log::warn!("tools the assistant wrote are not signed: no keychain key"),
    }
}

/// At start, when the assistant has written tools: their signing key, before any runs.
pub fn unlock_signing(config_dir: &Path) {
    if dir(config_dir).is_dir() {
        tauri::async_runtime::block_on(ensure_signing_key(config_dir));
    }
}

/// HMAC-SHA256 (RFC 2104) of `message` under `key`, as hex.
fn hmac(key: &[u8; 32], message: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut padded = [0u8; 64];
    padded[..32].copy_from_slice(key);
    let pad = |byte: u8| padded.iter().map(|k| k ^ byte).collect::<Vec<u8>>();
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What the signature covers: what the user approved, what the tool may reach, and everything
/// the monitor and History go by, so a file changed outside the app can neither widen a tool nor
/// quiet the monitor.
fn signed_part(tool: &ScriptTool) -> Vec<u8> {
    let mut part = json!({
        "name": tool.name,
        "script": tool.script,
        "description": tool.description,
        "parameters": tool.parameters,
        "approvedHash": tool.approved_hash,
        "status": tool.status,
        "granted": tool.granted,
        "grantedUntil": tool.granted_until,
        "expires": tool.expires,
        "approvedAt": tool.approved_at,
        "checks": tool.checks,
        "setAside": tool.set_aside,
        "restoredAt": tool.restored_at,
        "monitorRevertedAt": tool.monitor_reverted_at,
        "createdAt": tool.created_at,
        "pendingCheck": tool.pending_check,
        "requested": tool.requested,
    });
    // Numbered saves only: a tool signed before saves were numbered keeps verifying.
    if tool.seq > 0 {
        part["seq"] = json!(tool.seq);
    }
    serde_json::to_vec(&part).unwrap_or_default()
}

fn sign(config_dir: &Path, tool: &ScriptTool) -> Option<String> {
    let key = signing_key(config_dir)?;
    Some(hmac(&key, &signed_part(tool)))
}

/// Whether the app signed this state of a tool, at some time: for History entries and other
/// earlier states. True when there is no key to check with.
fn signed_by_app(config_dir: &Path, tool: &ScriptTool) -> bool {
    match sign(config_dir, tool) {
        None => true,
        Some(expected) => {
            tool.signature.as_deref() == Some(expected.as_str())
                || inherited()
                    .read()
                    .unwrap_or_else(|p| p.into_inner())
                    .iter()
                    .find(|(dir, _)| dir == config_dir)
                    .is_some_and(|(_, live)| {
                        tool.signature.as_deref() == Some(hmac(live, &signed_part(tool)).as_str())
                    })
        }
    }
}

/// Whether a tool's file is as the app last saved it: signed by the app, and the latest state it
/// signed, not an earlier one put back. True when there is no key to check with.
fn verifies(config_dir: &Path, tool: &ScriptTool) -> bool {
    signed_by_app(config_dir, tool)
        && match ledger()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .latest
            .get(&(config_dir.to_path_buf(), tool.name.clone()))
        {
            None => true,
            // The recorded latest save, or a later one whose record never reached the keychain.
            Some(latest) => {
                tool.signature.as_deref() == Some(latest.sig.as_str()) || tool.seq > latest.seq
            }
        }
}

/// A regression check: these arguments must keep giving this result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub args: Value,
    pub expected: String,
}

fn dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("script-tools")
}

/// Lowercase letters, digits and underscores, 3 to 40 of them, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    (3..=40).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn path(config_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(
            "A tool name uses lowercase letters, digits and underscores, like `count_words`."
                .into(),
        );
    }
    Ok(dir(config_dir).join(format!("{name}.json")))
}

/// The full SHA-256 of `content`, as hex: what an approval pins.
pub fn hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// What the user approves: the script, and the description and parameters the model reads to
/// decide when and how to call it. A change to any of them needs a new approval.
pub fn approval_hash(tool: &ScriptTool) -> String {
    hash(
        &serde_json::to_string(&json!({
            "script": tool.script,
            "description": tool.description,
            "parameters": tool.parameters,
        }))
        .unwrap_or_default(),
    )
}

pub fn load(config_dir: &Path, name: &str) -> Option<ScriptTool> {
    let text = crate::chat::at_rest::read_to_string(path(config_dir, name).ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn list(config_dir: &Path) -> Vec<ScriptTool> {
    let mut out: Vec<ScriptTool> = std::fs::read_dir(dir(config_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let text = crate::chat::at_rest::read_to_string(e.path()).ok()?;
            serde_json::from_str(&text).ok()
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Saves a change to a tool, re-signed. A tool whose file was changed outside the app is not
/// changed further this way: its grants would be signed as if the app had made them. Only paths
/// that show the user the tool again (promotion) or take its access away use [`save_resigned`].
pub(crate) fn save(config_dir: &Path, tool: &ScriptTool) -> Result<(), String> {
    if let Some(on_disk) = load(config_dir, &tool.name) {
        if !verifies(config_dir, &on_disk) {
            return Err(format!(
                "`{}`'s file was changed outside the app, so it was not changed further; promote it again so the user sees what it is now.",
                tool.name
            ));
        }
    }
    save_resigned(config_dir, tool)
}

fn save_resigned(config_dir: &Path, tool: &ScriptTool) -> Result<(), String> {
    let path = path(config_dir, &tool.name)?;
    // Signed as saved by the app: what the user approved, and what it may reach, numbered above
    // every save before it.
    let previous = load(config_dir, &tool.name);
    let mut tool = tool.clone();
    // Numbered from what the app recorded or signed, never from a number written outside it.
    tool.seq = recorded_seq(config_dir, &tool.name)
        .max(
            previous
                .as_ref()
                .filter(|p| signed_by_app(config_dir, p))
                .map_or(0, |p| p.seq),
        )
        .saturating_add(1);
    tool.signature = sign(config_dir, &tool);
    let tool = &tool;
    std::fs::create_dir_all(dir(config_dir)).map_err(|e| e.to_string())?;
    if let Some(previous) = previous {
        crate::chat::versions::keep(
            config_dir,
            "tools",
            &format!("script-{}", tool.name),
            &previous,
            tool,
        )?;
    }
    let text = serde_json::to_string_pretty(tool).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(path, &text).map_err(|e| e.to_string())?;
    // The last state the app signed, kept apart from the file the tool runs from, and its
    // signature recorded as the latest.
    if let Some(signature) = &tool.signature {
        record_latest(config_dir, &tool.name, signature, tool.seq);
        let copy = signed_copy(config_dir, &tool.name)?;
        if let Some(parent) = copy.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::chat::at_rest::write(copy, &text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The tools on offer: the active ones, as the model sees them.
pub fn definitions(config_dir: &Path) -> Vec<ChatTool> {
    list(config_dir)
        .into_iter()
        // Only what the user approved and the app saved: a tool whose file was changed outside
        // the app is not offered, so its description never reaches the model.
        .filter(|t| {
            t.status == Status::Active
                && t.approved_hash.as_deref() == Some(approval_hash(t).as_str())
                && verifies(config_dir, t)
        })
        .map(|t| ChatTool {
            name: cortex_kernel::tool::safe_name(&format!("{CONNECTOR}__{}", t.name)),
            description: format!(
                "{} (A tool you wrote; it runs in a sandbox.)",
                t.description
            ),
            parameters: t.parameters.clone(),
            connector_id: Some(CONNECTOR.into()),
            tool_name: t.name.clone(),
            // Without network access or writes, its own or through the tools it calls, it can only
            // compute and read: nothing changes and nothing leaves the machine.
            read_only: !has_effects(config_dir, &t),
        })
        .collect()
}

/// Whether a tool can reach the network or write files, itself or through any tool it may call,
/// however deep: a tool's effect is the union of its callees' effects.
pub fn has_effects(config_dir: &Path, tool: &ScriptTool) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![tool.clone()];
    while let Some(tool) = queue.pop() {
        if !seen.insert(tool.name.clone()) {
            continue;
        }
        if !tool.granted.http_hosts.is_empty() || !tool.granted.write_prefixes.is_empty() {
            return true;
        }
        queue.extend(
            tool.granted
                .tools
                .iter()
                .filter_map(|callee| load(config_dir, callee)),
        );
    }
    false
}

/// What `create_tool` asks for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Draft {
    name: String,
    description: String,
    #[serde(default)]
    parameters: Option<Value>,
    script: String,
    #[serde(default)]
    test_args: Option<Value>,
    #[serde(default)]
    http_hosts: Vec<String>,
    #[serde(default)]
    read_prefixes: Vec<String>,
    #[serde(default)]
    write_prefixes: Vec<String>,
    /// Vault secrets for its requests, as `host=SECRET`.
    #[serde(default)]
    credentials: Vec<String>,
    /// Other approved tools it calls with `tool(name, args)`.
    #[serde(default)]
    tools: Vec<String>,
    /// Save it even though a similar tool exists.
    #[serde(default)]
    new_anyway: bool,
}

/// An existing tool that does what `description` says, if one does: the kernel's rule is to
/// search before creating and to update a tool rather than add a near-duplicate.
fn near_duplicate<'a>(
    tools: &'a [ScriptTool],
    name: &str,
    description: &str,
) -> Option<&'a ScriptTool> {
    use std::collections::HashSet;
    let words = |t: &str| -> HashSet<String> {
        crate::chat::retrieval::tokens(t)
            .into_iter()
            .filter(|w| w.chars().count() > 3 && !matches!(w.as_str(), "tool" | "tools"))
            .collect()
    };
    let new = words(&format!("{} {description}", name.replace('_', " ")));
    if new.is_empty() {
        return None;
    }
    tools
        .iter()
        .filter(|t| t.name != name && t.status != Status::Deprecated)
        .map(|t| {
            let old = words(&format!("{} {}", t.name.replace('_', " "), t.description));
            let shared = new.intersection(&old).count() as f64;
            let union = new.union(&old).count().max(1) as f64;
            (t, shared / union)
        })
        .filter(|(_, overlap)| *overlap >= 0.5)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t)
}

/// Runs `script` on each check; the failures, in words.
async fn failing_checks(script: &str, checks: &[Check]) -> Vec<String> {
    let mut failed = Vec::new();
    for check in checks {
        // A fixed clock and seeded randomness, as when the check was recorded: a tool that reads
        // the date or draws random numbers still gives the same result tomorrow.
        let (result, _) = sandbox::run(
            script,
            &check.args.to_string(),
            &Capabilities::default(),
            Limits {
                deterministic: true,
                ..Limits::default()
            },
            None,
        )
        .await;
        match result {
            Ok(out) if out == check.expected => {}
            Ok(out) => failed.push(format!(
                "{} gave {} instead of {}",
                check.args,
                out.chars().take(200).collect::<String>(),
                check.expected.chars().take(200).collect::<String>()
            )),
            Err(e) => failed.push(format!("{} failed: {e}", check.args)),
        }
    }
    failed
}

/// Names a written tool may not take: the app's own tools.
fn taken(name: &str) -> bool {
    crate::chat::tool_names::NAMES.contains(&name)
        || [
            crate::chat::tool_names::DELEGATE,
            crate::chat::tool_names::CANVAS,
            crate::chat::tool_names::NOTIFY,
            CREATE_TOOL,
            PROMOTE_TOOL,
            RETIRE_TOOL,
            REQUEST_CAPABILITY,
        ]
        .contains(&name)
}

pub const CREATE_TOOL: &str = "create_tool";
pub const PROMOTE_TOOL: &str = "promote_tool";
pub const RETIRE_TOOL: &str = "retire_tool";
pub const REQUEST_CAPABILITY: &str = "request_capability";
pub const RUN_SCRIPT: &str = "run_script";

/// The kernel's `run(code, limits)`: a one-off JavaScript program in the sandbox, with no access
/// to anything, within the limits asked for (capped at the defaults). Nothing is saved.
pub async fn run_once(config_dir: &Path, args: &Value) -> Result<String, String> {
    sandbox::cache_in(config_dir);
    let code = args
        .get("code")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .ok_or("Give the program as `code`.")?;
    let input = args.get("args").cloned().unwrap_or_else(|| json!({}));
    let defaults = Limits::default();
    let limits = Limits {
        timeout: args
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .map_or(defaults.timeout, |s| {
                std::time::Duration::from_secs(s.clamp(1, defaults.timeout.as_secs()))
            }),
        memory_bytes: args
            .get("memory_mb")
            .and_then(Value::as_u64)
            .map_or(defaults.memory_bytes, |mb| {
                ((mb.clamp(16, 128) as usize) << 20).min(defaults.memory_bytes)
            }),
        ..defaults
    };
    let (result, _logs) = sandbox::run(
        code,
        &input.to_string(),
        &Capabilities::default(),
        limits,
        None,
    )
    .await;
    result
        .map(|out| cortex_kernel::privacy::mask_secrets(&out).0)
        .map_err(|e| e.to_string())
}

/// `create_tool`: saves (or replaces) a candidate and runs it once on its test arguments, with no
/// capabilities. A promoted tool that is rewritten goes back to being a candidate.
pub async fn create(config_dir: &Path, conversation: &str, args: &Value) -> Result<String, String> {
    sandbox::cache_in(config_dir);
    // Signed from its first save: a tool written before the key was loaded would otherwise look,
    // once it is, like a file changed outside the app.
    #[cfg(not(test))]
    ensure_signing_key(config_dir).await;
    let draft: Draft = serde_json::from_value(args.clone())
        .map_err(|e| format!("The tool is not described right: {e}"))?;
    let name = draft.name.trim().to_string();
    if taken(&name) {
        return Err(format!(
            "`{name}` is one of the app's own tools; choose another name."
        ));
    }
    if draft.description.trim().is_empty() {
        return Err("Describe what the tool does and when to use it.".into());
    }
    if !draft.script.contains("function main") {
        return Err("The script must define `function main(args)` and return the result.".into());
    }
    let parameters = draft
        .parameters
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    if !parameters.is_object() {
        return Err("`parameters` must be a JSON schema object.".into());
    }
    let existing = list(config_dir);
    if !draft.new_anyway && !existing.iter().any(|t| t.name == name) {
        if let Some(similar) = near_duplicate(&existing, &name, &draft.description) {
            return Err(format!(
                "`{}` already looks like this tool ({}). Update it by calling create_tool with name \
                 `{}`, or, if this is really a different tool, call again with new_anyway: true.",
                similar.name, similar.description, similar.name
            ));
        }
    }
    let previous = existing.into_iter().find(|t| t.name == name);
    // A rewrite keeps the checks and set-aside list the app signed: for a file changed outside
    // the app, those of the last state the app signed.
    let signed_previous = previous.as_ref().and_then(|t| {
        if verifies(config_dir, t) {
            Some(t.clone())
        } else {
            last_signed(config_dir, &name)
        }
    });
    let checks = signed_previous
        .as_ref()
        .map(|t| t.checks.clone())
        .unwrap_or_default();
    let tool = ScriptTool {
        name: name.clone(),
        description: draft.description.trim().to_string(),
        parameters,
        script: draft.script,
        requested: Capabilities {
            http_hosts: draft.http_hosts,
            read_prefixes: draft.read_prefixes,
            write_prefixes: draft.write_prefixes,
            tools: draft.tools,
            credentials: draft.credentials,
        },
        granted: Capabilities::default(),
        status: Status::Candidate,
        approved_hash: None,
        created_at: Utc::now(),
        provenance: (!conversation.is_empty()).then(|| conversation.to_string()),
        checks,
        pending_check: None,
        granted_until: None,
        signature: None,
        expires: Default::default(),
        monitor_reverted_at: None,
        approved_at: None,
        // A rewrite keeps what the user set aside; only approving a set-aside version clears it.
        // (From what the app signed: a file changed outside the app plants nothing.)
        set_aside: signed_previous
            .as_ref()
            .map(|t| t.set_aside.clone())
            .unwrap_or_default(),
        restored_at: None,
        seq: 0,
    };
    let mut tool = tool;
    save_resigned(config_dir, &tool)?;
    crate::chat::traces::register(
        config_dir,
        "script_tool",
        &name,
        Some(&hash(&tool.script)),
        tool.provenance.as_deref(),
        Status::Candidate.as_str(),
    );
    // The test run: no capabilities, whatever the tool asked for, and deterministic, since what
    // it gives becomes the tool's regression check.
    let test_args = draft.test_args.unwrap_or_else(|| json!({})).to_string();
    let (result, logs) = sandbox::run(
        &tool.script,
        &test_args,
        &Capabilities::default(),
        Limits {
            deterministic: true,
            ..Limits::default()
        },
        None,
    )
    .await;
    let test = match result {
        Ok(output) => {
            tool.pending_check = Some(Check {
                args: serde_json::from_str(&test_args).unwrap_or(Value::Null),
                expected: output.clone(),
            });
            save_resigned(config_dir, &tool)?;
            json!({ "ok": true, "output": output.chars().take(4_000).collect::<String>() })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    };
    // A rewrite must keep passing the checks the tool already had.
    let regressions = failing_checks(&tool.script, &tool.checks).await;
    Ok(json!({
        "saved": name,
        "status": "candidate",
        "test": test,
        "checks": { "count": tool.checks.len(), "failing": regressions },
        "logs": logs.len(),
        "next": "Fix the script with create_tool if the test is wrong. When it works, call promote_tool so the user can review and approve it; until then it is not offered. Its test ran with no network or file access, whatever it asked for."
    })
    .to_string())
}

/// What the approval card shows for a promotion: the script, the description the model will read
/// and the capabilities asked for.
pub fn review(tool: &ScriptTool) -> Value {
    json!({
        "tool": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "access": if tool.requested.is_empty() {
            "none: it can only compute"
        } else {
            "the network hosts, folders and tools below"
        },
        "network": tool.requested.http_hosts,
        "reads": tool.requested.read_prefixes,
        "writes": tool.requested.write_prefixes,
        "credentials": tool.requested.credentials,
        "calls": tool.requested.tools,
        "script": tool.script,
    })
}

/// Makes an approved candidate active: its regression checks must pass, then its capabilities are
/// granted (for `days`, or until revoked) and its script pinned. Its last good test run becomes a
/// check.
/// [`promote`], only if the tool is still exactly what the user was shown: its script,
/// description, parameters and the access it asks for. Anything changed in between (another call
/// in the same step, another chat) is refused, not approved unseen.
pub async fn promote_as_shown(
    config_dir: &Path,
    shown: &ScriptTool,
    days: Option<i64>,
) -> Result<String, String> {
    promote_with(config_dir, &shown.name, days, Some(shown)).await
}

pub async fn promote(config_dir: &Path, name: &str, days: Option<i64>) -> Result<String, String> {
    promote_with(config_dir, name, days, None).await
}

/// Where the last state the app signed for each tool is kept: History keeps only the states a
/// save replaced, so once a tool's own file is changed outside the app, this copy is the one
/// that still says what the app last signed.
fn signed_copy(config_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let file = path(config_dir, name)?;
    Ok(chat_dir(config_dir)
        .join("script-tools-signed")
        .join(file.file_name().ok_or("bad tool name")?))
}

/// The last state of a tool the app signed: its signed copy, else the newest History entry that
/// verifies.
fn last_signed(config_dir: &Path, name: &str) -> Option<ScriptTool> {
    let copy = signed_copy(config_dir, name)
        .ok()
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<ScriptTool>(&text).ok())
        .filter(|t| t.name == name && verifies(config_dir, t));
    copy.or_else(|| {
        crate::chat::versions::list(config_dir, "tools", &format!("script-{name}"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<ScriptTool>(v.value).ok())
            .find(|v| v.name == name && signed_by_app(config_dir, v))
    })
}

/// A last signed state's regression checks, its pending one included.
fn signed_checks(last: &ScriptTool) -> Vec<Check> {
    let mut checks = last.checks.clone();
    if let Some(check) = last.pending_check.clone() {
        if !checks.iter().any(|c| c.args == check.args) {
            checks.push(check);
        }
    }
    checks
}

/// For a file changed outside the app, what the signature vouched for and the approval card does
/// not show (its regression checks, the versions set aside, the monitor's and History's dates) is
/// taken from the last state the app signed, so a promotion never signs what was written around
/// it. With no signed state left, the promotion is refused: the tool is written again instead,
/// and tested afresh.
fn take_vouched(config_dir: &Path, tool: &mut ScriptTool) -> Result<(), String> {
    if verifies(config_dir, tool) {
        return Ok(());
    }
    let Some(last) = last_signed(config_dir, &tool.name) else {
        // Never signed at all (written before the key was loaded, or by a build that did not
        // sign): nothing was vouched for, so nothing is taken away; it is promoted as written,
        // with no rollback or restore date of its own.
        if tool.signature.is_none() && !ever_signed(config_dir, &tool.name) {
            tool.monitor_reverted_at = None;
            tool.restored_at = None;
            return Ok(());
        }
        return Err(format!(
            "`{}`'s file was changed outside the app and no state the app signed remains to take its regression checks from, so it was not promoted; write it again with create_tool so it is tested afresh.",
            tool.name
        ));
    };
    tool.checks = signed_checks(&last);
    tool.pending_check = None;
    tool.set_aside = last.set_aside;
    tool.monitor_reverted_at = last.monitor_reverted_at;
    tool.restored_at = last.restored_at;
    tool.approved_at = last.approved_at;
    tool.created_at = last.created_at;
    Ok(())
}

/// Saves a change that only takes something away (demoting, revoking, retiring). A file changed
/// outside the app is saved as it is, unsigned: signing it would vouch for what was written
/// around it, and it stays unoffered until promoted, which takes those fields from the last state
/// the app signed.
fn save_taking_away(config_dir: &Path, tool: &ScriptTool) -> Result<(), String> {
    let on_disk = load(config_dir, &tool.name);
    if on_disk.as_ref().is_none_or(|t| verifies(config_dir, t)) {
        return save_resigned(config_dir, tool);
    }
    let path = path(config_dir, &tool.name)?;
    let mut tool = tool.clone();
    tool.signature = None;
    if let Some(previous) = on_disk {
        crate::chat::versions::keep(
            config_dir,
            "tools",
            &format!("script-{}", tool.name),
            &previous,
            &tool,
        )?;
    }
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(&tool).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

async fn promote_with(
    config_dir: &Path,
    name: &str,
    days: Option<i64>,
    shown: Option<&ScriptTool>,
) -> Result<String, String> {
    sandbox::cache_in(config_dir);
    #[cfg(not(test))]
    ensure_signing_key(config_dir).await;
    let mut first = load(config_dir, name).ok_or("There is no tool by that name.")?;
    take_vouched(config_dir, &mut first)?;
    let failing = failing_checks(&first.script, &first.checks).await;
    if !failing.is_empty() {
        return Err(format!(
            "`{name}` fails its regression checks, so it was not promoted: {}",
            failing.join("; ")
        ));
    }
    // Loaded again after the waits above, and promoted only if it is still what was checked and,
    // for a card, what the user was shown: nothing written in between is approved unseen.
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    take_vouched(config_dir, &mut tool)?;
    let expected = shown.unwrap_or(&first);
    if approval_hash(&tool) != approval_hash(expected)
        || tool.requested != expected.requested
        || approval_hash(&tool) != approval_hash(&first)
    {
        return Err(format!(
            "`{name}` changed after the user was shown it, so it was not promoted; promote it again so they see what it is now."
        ));
    }
    if let Some(check) = tool.pending_check.take() {
        if !tool.checks.iter().any(|c| c.args == check.args) {
            tool.checks.push(check);
        }
    }
    tool.granted_until = days
        .filter(|d| *d > 0)
        .map(|d| Utc::now() + chrono::Duration::days(d));
    tool.granted = tool.requested.clone();
    tool.expires.clear();
    tool.status = Status::Active;
    // A version brought back from History and not approved since (a draft, say) has its own week
    // to build from this approval before it is compared week over week, not the weeks of the
    // versions it set aside. A restore already approved keeps its date and so its 14 days; any
    // other approval is compared with the version it replaces.
    let restore_unapproved = tool
        .restored_at
        .is_some_and(|at| tool.approved_at.is_none_or(|approved| at > approved));
    if restore_unapproved {
        tool.restored_at = Some(Utc::now());
    }
    tool.approved_at = Some(Utc::now());
    tool.approved_hash = Some(approval_hash(&tool));
    // An approval is judged on its own calls from now: a rollback before it holds nothing back.
    tool.monitor_reverted_at = None;
    // Approved again by the user: no longer set aside.
    let approved = approval_hash(&tool);
    tool.set_aside.retain(|h| h != &approved);
    // The user saw it on the card and approved it: signed afresh.
    save_resigned(config_dir, &tool)?;
    crate::chat::traces::register(
        config_dir,
        "script_tool",
        name,
        tool.approved_hash.as_deref(),
        None,
        Status::Active.as_str(),
    );
    Ok(format!(
        "`{name}` is now a tool, available in every chat as script__{name}."
    ))
}

/// Takes a tool off offer until it is promoted again: the lifecycle does this to one that keeps
/// failing.
pub fn demote(config_dir: &Path, name: &str) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    // Judged on the file as it is, before this change.
    forget_approval_if_tampered(config_dir, &mut tool);
    tool.status = Status::Candidate;
    tool.granted = Capabilities::default();
    tool.expires.clear();
    save_taking_away(config_dir, &tool)?;
    crate::chat::traces::set_status(config_dir, "script_tool", name, Status::Candidate.as_str());
    Ok(())
}

/// Takes back a tool's capabilities; it stays on offer, able only to compute.
pub fn revoke(config_dir: &Path, name: &str) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    // Judged on the file as it is, before this change: a tool the app saved stays on offer.
    forget_approval_if_tampered(config_dir, &mut tool);
    tool.granted = Capabilities::default();
    tool.requested = Capabilities::default();
    tool.granted_until = None;
    tool.expires.clear();
    save_taking_away(config_dir, &tool)
}

/// Before a path that re-signs a file (taking access away, retiring), a file changed outside the
/// app loses its approval: it is signed only as a candidate the user must approve again, never
/// as the active tool it now claims to be.
fn forget_approval_if_tampered(config_dir: &Path, tool: &mut ScriptTool) {
    if !verifies(config_dir, tool) {
        tool.approved_hash = None;
        tool.approved_at = None;
        if tool.status == Status::Active {
            tool.status = Status::Candidate;
        }
    }
}

/// What a capability request would change, for its card: what the tool may reach now, what lapsed
/// (and is not renewed), and what it asks for.
pub fn access_card(tool: &ScriptTool, wanted: &Capabilities) -> Value {
    let active = active_grants(tool, Utc::now());
    let lapsed: Vec<String> = keyed(&tool.granted)
        .into_iter()
        .filter(|k| !keyed(&active).contains(k))
        .collect();
    json!({ "hasNow": active, "lapsed": lapsed, "asksFor": wanted })
}

/// More access for a tool, once the user approved the card built from `shown`: what it asks for,
/// for `days` or until revoked, added to what it may still reach. Each grant keeps its own
/// lapse: a short request does not shorten a lasting grant, and a lapsed grant is not renewed.
/// Refused when the tool changed after the card was shown.
pub fn extend(
    config_dir: &Path,
    shown: &ScriptTool,
    wanted: &Capabilities,
    days: Option<i64>,
) -> Result<(), String> {
    let mut tool = load(config_dir, &shown.name).ok_or("There is no tool by that name.")?;
    let unchanged = approval_hash(&tool) == approval_hash(shown)
        && tool.granted == shown.granted
        && tool.requested == shown.requested
        && tool.expires == shown.expires
        && tool.granted_until == shown.granted_until;
    if !unchanged {
        return Err(format!(
            "`{}` changed after the user was shown the request, so nothing was granted; ask again.",
            shown.name
        ));
    }
    let now = Utc::now();
    let active = active_grants(&tool, now);
    // Lapses that came from the tool-wide date become each grant's own.
    let mut expires = std::collections::BTreeMap::new();
    for key in keyed(&active) {
        if let Some(until) = tool.expires.get(&key).copied().or(tool.granted_until) {
            expires.insert(key, until);
        }
    }
    let until = days
        .filter(|d| *d > 0)
        .map(|d| now + chrono::Duration::days(d));
    let had = keyed(&active);
    for key in keyed(wanted) {
        // Asked for again, a grant keeps whichever lasts longer: lasting beats any date.
        let lasting_already = had.contains(&key) && !expires.contains_key(&key);
        match until {
            _ if lasting_already => {}
            Some(until) => {
                let later = expires.get(&key).map_or(until, |&e| e.max(until));
                expires.insert(key, later);
            }
            None => {
                expires.remove(&key);
            }
        }
    }
    tool.granted = active.with(wanted);
    tool.requested = tool.granted.clone();
    tool.expires = expires;
    tool.granted_until = None;
    save(config_dir, &tool)
}

/// Brings back an earlier version from History. Its script comes back; its access does not grow
/// past what the tool may reach now, and a version that does not verify (written into the history
/// from outside) comes back only as a candidate, to be promoted again.
pub fn restore_version(config_dir: &Path, mut old: ScriptTool) -> Result<(), String> {
    let now = Utc::now();
    let current = load(config_dir, &old.name);
    let limit = current
        .as_ref()
        .filter(|c| verifies(config_dir, c))
        .map(|c| active_grants(c, now))
        .unwrap_or_default();
    let approved = signed_by_app(config_dir, &old)
        && old.approved_hash.as_deref() == Some(approval_hash(&old).as_str());
    if !approved {
        old.status = Status::Candidate;
        old.approved_hash = None;
    }
    old.granted = old.granted.within(&limit);
    if let Some(current) = &current {
        old.expires = current.expires.clone();
        old.granted_until = current.granted_until;
    }
    // Every version approved after the one brought back, the current one included, is set
    // aside by the user's choice: the monitor never rolls forward to any of them.
    // An approved version is placed by when it was approved; a draft, or a version that does not
    // verify, by when it was written: the approvals after that are set aside, never those before.
    // A History entry changed outside the app proves nothing about when it was written: every
    // approval is set aside, and its checks are the last signed ones, never its own.
    let verified = signed_by_app(config_dir, &old);
    let restored_from = if approved {
        old.approved_at.or(Some(old.created_at))
    } else if verified {
        Some(old.created_at)
    } else {
        Some(DateTime::<Utc>::MIN_UTC)
    };
    let signed_current = current
        .as_ref()
        .filter(|c| verifies(config_dir, c))
        .cloned()
        .or_else(|| last_signed(config_dir, &old.name));
    if !verified {
        old.checks = signed_current
            .as_ref()
            .map(signed_checks)
            .unwrap_or_default();
        old.pending_check = None;
        // Its dates are not taken on its word: as far as the app knows, it was written now.
        old.created_at = now;
        old.approved_at = None;
    }
    let restored = approval_hash(&old);
    // What the user set aside, as the app last signed it: a current file changed outside the app
    // drops nothing from it.
    let mut set_aside = signed_current
        .as_ref()
        .map(|c| c.set_aside.clone())
        .unwrap_or_default();
    let mut later: Vec<String> =
        crate::chat::versions::list(config_dir, "tools", &format!("script-{}", old.name))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<ScriptTool>(v.value).ok())
            .filter(|v| {
                v.status == Status::Active
                    && restored_from.is_some_and(|from| v.approved_at.is_some_and(|at| at > from))
            })
            .map(|v| approval_hash(&v))
            .collect();
    if let Some(current) = signed_current
        .as_ref()
        .filter(|c| c.status == Status::Active)
    {
        later.push(approval_hash(current));
    }
    for hash in later {
        if hash != restored && !set_aside.contains(&hash) {
            set_aside.push(hash);
        }
    }
    // Brought back, it is no longer set aside itself.
    set_aside.retain(|h| h != &restored);
    old.set_aside = set_aside;
    // Back in use from now, by the user's choice: judged on its own calls from here, never on
    // the calls of the versions it set aside. A draft brought back carries the mark into its
    // approval.
    if old.status == Status::Active {
        old.approved_at = Some(now);
    }
    old.restored_at = Some(now);
    old.monitor_reverted_at = None;
    save_resigned(config_dir, &old)?;
    crate::chat::traces::register(
        config_dir,
        "script_tool",
        &old.name,
        old.approved_hash.as_deref(),
        None,
        old.status.as_str(),
    );
    Ok(())
}

/// What the daily regression checks did.
#[derive(Debug, Default, PartialEq)]
pub struct Checked {
    /// Brought back to their last approved version that still passes.
    pub reverted: Vec<String>,
    /// Sent back to candidate: no earlier version passes either.
    pub demoted: Vec<String>,
}

/// Runs every active tool's regression checks. A tool that fails one goes back to its previous
/// approved version when that version passes them; otherwise it goes back to candidate.
pub async fn check_all(config_dir: &Path) -> Checked {
    sandbox::cache_in(config_dir);
    let mut done = Checked::default();
    for tool in list(config_dir)
        .into_iter()
        .filter(|t| t.status == Status::Active && !t.checks.is_empty())
    {
        if failing_checks(&tool.script, &tool.checks).await.is_empty() {
            continue;
        }
        if revert(config_dir, &tool.name, &tool.checks).await == Ok(true) {
            done.reverted.push(tool.name);
        } else if demote(config_dir, &tool.name).is_ok() {
            done.demoted.push(tool.name);
        }
    }
    done
}

/// Brings a tool back to its newest earlier version that the user approved, runs a different
/// script, and passes `checks`. The version it replaces is kept, so this too can be undone.
/// `Ok(false)` when there is no such version.
pub async fn revert(config_dir: &Path, name: &str, checks: &[Check]) -> Result<bool, String> {
    let current = load(config_dir, name).ok_or("There is no tool by that name.")?;
    for version in crate::chat::versions::list(config_dir, "tools", &format!("script-{name}"))? {
        let Ok(old) = serde_json::from_value::<ScriptTool>(version.value) else {
            continue;
        };
        // Only a version the app saved as approved: its hash pins its script, and its
        // signature shows nobody wrote it into the history from outside.
        let approved = old.status == Status::Active
            && old.approved_hash.as_deref() == Some(approval_hash(&old).as_str())
            && signed_by_app(config_dir, &old);
        // Never forward to a version the user set aside by restoring an earlier one.
        if !approved
            || old.script == current.script
            || current.set_aside.contains(&approval_hash(&old))
        {
            continue;
        }
        if failing_checks(&old.script, checks).await.is_empty() {
            // Checking took a while: a revoke or any other change meanwhile wins, and the
            // rollback waits for the next pass.
            let latest = load(config_dir, name).ok_or("There is no tool by that name.")?;
            if latest.granted != current.granted
                || latest.requested != current.requested
                || latest.expires != current.expires
                || latest.status != current.status
                || approval_hash(&latest) != approval_hash(&current)
            {
                return Ok(false);
            }
            // The script goes back; the access does not grow: it keeps only what the tool may
            // reach now, so a revoke or a lapse is never undone by a rollback.
            let mut back = old;
            back.granted = back.granted.within(&active_grants(&current, Utc::now()));
            back.requested = back.requested.within(&current.requested);
            back.expires = current.expires.clone();
            back.granted_until = current.granted_until;
            // Back in use from now: judged on its calls from here, and not rolled back again
            // within the week on the failures of the version it replaced.
            back.approved_at = Some(Utc::now());
            back.monitor_reverted_at = Some(Utc::now());
            back.set_aside = current.set_aside.clone();
            back.restored_at = None;
            save(config_dir, &back)?;
            crate::chat::traces::register(
                config_dir,
                "script_tool",
                name,
                back.approved_hash.as_deref(),
                None,
                back.status.as_str(),
            );
            return Ok(true);
        }
    }
    Ok(false)
}

/// Calls per tool per minute; past it, calls are refused until the minute passes.
const CALLS_PER_MINUTE: usize = 30;

/// Per data folder and tool: a trial's copy does not use up the live tool's minute.
type Calls = std::sync::Mutex<
    std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>,
>;

fn rate_calls() -> &'static Calls {
    static CALLS: std::sync::OnceLock<Calls> = std::sync::OnceLock::new();
    CALLS.get_or_init(Default::default)
}

/// Drops a folder's rate counters, for a folder that is going away (a trial's copy).
pub fn forget_rates(config_dir: &Path) {
    let prefix = format!("{}:", config_dir.display());
    rate_calls()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|key, _| !key.starts_with(&prefix));
}

fn within_rate(name: &str) -> bool {
    use std::time::{Duration, Instant};
    let mut calls = rate_calls().lock().unwrap_or_else(|p| p.into_inner());
    let recent = calls.entry(name.to_string()).or_default();
    let now = Instant::now();
    while recent
        .front()
        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
    {
        recent.pop_front();
    }
    if recent.len() >= CALLS_PER_MINUTE {
        return false;
    }
    recent.push_back(now);
    true
}

/// Hides a tool; promoting it again brings it back.
pub fn retire(config_dir: &Path, name: &str) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    tool.status = Status::Deprecated;
    // A file changed outside the app is retired with no access and no approval, and left
    // unsigned.
    if !verifies(config_dir, &tool) {
        tool.granted = Capabilities::default();
        tool.expires.clear();
        tool.approved_hash = None;
        tool.approved_at = None;
    }
    save_taking_away(config_dir, &tool)?;
    crate::chat::traces::set_status(config_dir, "script_tool", name, Status::Deprecated.as_str());
    Ok(())
}

tokio::task_local! {
    /// The run a tool call belongs to, set by the engine around the call: a written tool's runs,
    /// and those of the tools it calls, are recorded in that run's trace.
    static RUN: String;
}

/// Runs `call` as part of `run`'s trace.
pub async fn in_run<F: std::future::Future>(run: String, call: F) -> F::Output {
    RUN.scope(run, call).await
}

/// How deep tools may call tools: a script calling a tool that calls a tool, and no further.
const MAX_DEPTH: usize = 3;

/// Runs an active tool with its granted capabilities, after checking its script is the one the
/// user approved. `workspace` is the chat's folder for `read`. A script may call the other tools
/// its capabilities name, with the same checks, a few levels deep.
pub async fn call(
    config_dir: &Path,
    name: &str,
    args: &Value,
    workspace: Option<&Path>,
) -> Result<String, String> {
    call_at(
        config_dir.to_path_buf(),
        name.to_string(),
        args.clone(),
        workspace.map(Path::to_path_buf),
        0,
    )
    .await
}

fn call_at(
    config_dir: PathBuf,
    name: String,
    args: Value,
    workspace: Option<PathBuf>,
    depth: usize,
) -> futures::future::BoxFuture<'static, Result<String, String>> {
    Box::pin(async move {
        sandbox::cache_in(&config_dir);
        if depth >= MAX_DEPTH {
            return Err(format!(
                "`{name}` is called too deep; tools call tools {MAX_DEPTH} levels at most."
            ));
        }
        let tool = load(&config_dir, &name).ok_or("That tool was removed.")?;
        if tool.status != Status::Active {
            return Err("That tool is not active; promote it first.".into());
        }
        if tool.approved_hash.as_deref() != Some(approval_hash(&tool).as_str()) {
            return Err(
                "This tool's script, description or parameters changed since it was approved; promote it again.".into(),
            );
        }
        if !verifies(&config_dir, &tool) {
            return Err(
                "This tool's file was changed outside the app, so its approval no longer holds; promote it again.".into(),
            );
        }
        if !within_rate(&format!("{}:{name}", config_dir.display())) {
            return Err(format!(
                "`{name}` was called {CALLS_PER_MINUTE} times in the last minute; wait before calling it again."
            ));
        }
        // Each capability granted for a time lapses on its own; the tool can still compute.
        let granted = active_grants(&tool, Utc::now());
        let caller: sandbox::ToolCaller = {
            let (config_dir, workspace) = (config_dir.clone(), workspace.clone());
            std::sync::Arc::new(move |callee: String, input: String| {
                let (config_dir, workspace) = (config_dir.clone(), workspace.clone());
                Box::pin(async move {
                    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
                    call_at(config_dir, callee, input, workspace, depth + 1).await
                })
            })
        };
        let workspace = workspace.map(|root| sandbox::Workspace {
            root,
            // Files the app sealed at rest are opened with its key, and what is written is sealed.
            read: |_, bytes| crate::chat::at_rest::open_bytes(bytes),
            write: |p, bytes| crate::chat::at_rest::seal_for(p, bytes),
        });
        // The vault secrets its granted credentials name, for hosts it may reach: the host
        // attaches them to requests, and they are masked in whatever comes back.
        let mut secrets = std::collections::HashMap::new();
        for credential in &granted.credentials {
            let Some((host, secret)) = credential.split_once('=') else {
                continue;
            };
            if granted
                .http_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(host))
            {
                if let Some(value) = crate::chat::vault::value(&config_dir, secret).await {
                    secrets.insert(secret.to_string(), value);
                }
            }
        }
        let shown: Vec<(String, String)> = secrets.clone().into_iter().collect();
        let started = std::time::Instant::now();
        let (result, logs) = sandbox::run_with_secrets(
            &tool.script,
            &args.to_string(),
            &granted,
            Limits::default(),
            workspace,
            Some(caller),
            secrets,
        )
        .await;
        // Its run, ok or not, in the trace: what the lifecycle and the monitor judge it by. A call
        // from another tool counts too, so a tool used only that way is not retired as unused.
        crate::chat::traces::tool_call(
            &config_dir,
            &RUN.try_with(Clone::clone).unwrap_or_default(),
            &format!("{CONNECTOR}__{name}"),
            started.elapsed().as_millis() as u64,
            result.is_ok(),
        );
        // The tool's own lines are untrusted text: they go to the log file, never to the model.
        for line in logs {
            log::info!("[script tool {name}] {line}");
        }
        // What it returns is scanned for secrets (keys, tokens, passwords) before anything reads
        // it, whatever host it fetched them from.
        // Masked whether it succeeded or failed: an error can quote a response, and with it a
        // credential. A callee's result is masked before it reaches its caller.
        let mask = |text: String| {
            let text = crate::chat::vault::mask(&text, &shown);
            let (masked, found) = cortex_kernel::privacy::mask_secrets(&text);
            if found > 0 {
                log::warn!("[script tool {name}] {found} secret(s) masked in its output");
            }
            masked
        };
        let result = result.map(mask).map_err(|e| mask(e.to_string()));
        // A call from another tool does not pass the chat's own record: it is logged here, so the
        // activity log shows every tool that ran.
        if depth > 0 {
            crate::chat::audit::record(
                &config_dir,
                crate::chat::audit::Entry {
                    at: Utc::now(),
                    kind: "tool".into(),
                    conversation: String::new(),
                    agent: None,
                    tool: format!("{CONNECTOR}__{name} (called by a tool)"),
                    arguments: crate::chat::audit::short(&args.to_string()),
                    outcome: if result.is_ok() { "ok" } else { "error" }.into(),
                    detail: crate::chat::audit::short(match &result {
                        Ok(text) | Err(text) => text,
                    }),
                    millis: started.elapsed().as_millis() as u64,
                },
            );
        }
        result
    })
}

/// The definitions of `create_tool` and `promote_tool`.
pub fn builtin_definitions() -> Vec<ChatTool> {
    let tool = |name: &str, description: &str, parameters: Value, read_only: bool| ChatTool {
        name: name.into(),
        description: description.into(),
        parameters,
        connector_id: None,
        tool_name: name.into(),
        read_only,
    };
    vec![
        tool(
            RUN_SCRIPT,
            "Run a short JavaScript program once, in a sandbox with no network, files or other \
             tools: for an exact calculation, parsing or reformatting. Define `function main(args)` \
             and return a JSON value (or print with console.log). Nothing is saved; for something \
             you will repeat, write a tool with create_tool.",
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "JavaScript defining function main(args), or a program that prints." },
                    "args": { "type": "object", "description": "The arguments main receives." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "memory_mb": { "type": "integer", "minimum": 16, "maximum": 128 }
                },
                "required": ["code"]
            }),
            true,
        ),
        tool(
            CREATE_TOOL,
            "Write a reusable tool for yourself when a task needs a small, exact computation or \
             fetch you would otherwise repeat: a JavaScript script that defines `function \
             main(args)` and returns a JSON value. It runs in a sandbox with no network or files \
             unless you ask for them: `fetch(method, url, body)` (https, listed hosts only) returns \
             {status, body}; `read(path)` reads a file in the chat's folder and `write(path, text)` \
             writes one where allowed; `log(text)` notes progress. It is saved as a candidate and tested once on `test_args`; call \
             promote_tool once it works, and the user decides. `tool(name, args)` calls another tool \
             you wrote, when you list it in `tools`.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "lowercase_with_underscores" },
                    "description": { "type": "string", "description": "What it does and when to use it, for your future self." },
                    "parameters": { "type": "object", "description": "JSON schema of the arguments." },
                    "script": { "type": "string", "description": "JavaScript defining function main(args)." },
                    "test_args": { "type": "object", "description": "Arguments for the test run." },
                    "http_hosts": { "type": "array", "items": { "type": "string" }, "description": "Exact host names it needs to reach over https, if any." },
                    "read_prefixes": { "type": "array", "items": { "type": "string" }, "description": "Folders of the chat's workspace it needs to read, if any." },
                    "write_prefixes": { "type": "array", "items": { "type": "string" }, "description": "Folders of the chat's workspace it needs to write, if any; `write(path, text)` writes there." },
                    "credentials": { "type": "array", "items": { "type": "string" }, "description": "Vault secrets its requests need, as host=SECRET_NAME; the app sends them as a bearer token to that host. The script never sees them." },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Other tools you wrote that it calls with tool(name, args), so their results never pass through the chat." },
                    "new_anyway": { "type": "boolean", "description": "Save it although a similar tool exists (you were told which); prefer updating that one." }
                },
                "required": ["name", "description", "script"]
            }),
            false,
        ),
        tool(
            PROMOTE_TOOL,
            "Ask the user to make a tool you wrote with create_tool available in every chat. They \
             see its script, description and access, and approve or decline.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "days": { "type": "integer", "minimum": 1, "description": "Grant its network and file access for this many days only." }
                },
                "required": ["name"]
            }),
            false,
        ),
        tool(
            RETIRE_TOOL,
            "Retire a tool you wrote that is no longer useful or was replaced; it stops being \
             offered and can be promoted again later.",
            json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
            false,
        ),
        tool(
            REQUEST_CAPABILITY,
            "Ask the user to let one of your tools reach more: https hosts or folders of the chat's \
             workspace, for a number of days, with the reason. Nothing changes until they approve.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "http_hosts": { "type": "array", "items": { "type": "string" } },
                    "read_prefixes": { "type": "array", "items": { "type": "string" } },
                    "write_prefixes": { "type": "array", "items": { "type": "string" } },
                    "credentials": { "type": "array", "items": { "type": "string" } },
                    "tools": { "type": "array", "items": { "type": "string" } },
                    "days": { "type": "integer", "minimum": 1 },
                    "reason": { "type": "string" }
                },
                "required": ["name", "reason"]
            }),
            false,
        ),
    ]
}

#[cfg(test)]
mod tests;
