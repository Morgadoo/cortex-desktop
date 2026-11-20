//! The key that signs approved tools, and the ledger of each tool's latest signature, so a file changed behind the app's back is not trusted.

use super::*;

pub(super) const SIGNING_KEY_NAME: &str = "tool-signing-key";

pub(super) type Keys = std::sync::RwLock<Vec<(PathBuf, [u8; 32])>>;

pub(super) fn keys() -> &'static Keys {
    &super::TRUST.keys
}

/// Signs and verifies the tools in `config_dir` with `key` from now on.
pub(in crate::chat) fn set_signing_key(config_dir: &Path, key: [u8; 32]) {
    let mut keys = keys().write().unwrap_or_else(|p| p.into_inner());
    keys.retain(|(dir, _)| dir != config_dir);
    keys.push((config_dir.to_path_buf(), key));
}

/// Live keys a trial's copy checks what it copied against; it never signs with them.
pub(super) fn inherited() -> &'static Keys {
    &super::TRUST.inherited
}

/// A trial's copy signs with a key of its own, derived from the live one and the copy's folder,
/// so nothing it signs verifies in the live folder; it checks what it copied against the live key.
pub(super) fn set_trial_keys(trial: &Path, live: [u8; 32]) {
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
pub(in crate::chat) fn share_signing_key(from: &Path, to: &Path) {
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
pub(in crate::chat) fn forget_signing_key(config_dir: &Path) {
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
pub(super) struct Ledger {
    pub(super) latest: std::collections::HashMap<(PathBuf, String), Latest>,
    pub(super) stores: Vec<(PathBuf, crate::tools::auth::Secrets)>,
    /// Trial copies: their record is never kept in the keychain, whatever key they hold.
    pub(super) trials: Vec<PathBuf>,
    /// Folders whose record could not be read from the keychain this session: no tool there is
    /// taken for one the app never signed.
    pub(super) unread: Vec<PathBuf>,
}

/// A tool's latest signature and the number of the save that made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Latest {
    pub(super) sig: String,
    pub(super) seq: u64,
}

pub(super) fn ledger() -> &'static std::sync::Mutex<Ledger> {
    &super::TRUST.ledger
}

/// The keychain entry holding a folder's record of latest signatures.
pub(super) fn latest_entry(config_dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
    let short: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("tool-signing-latest-{short}")
}

/// Whether the app ever signed a tool by this name in this folder. When the folder's record could
/// not be read, it may have: so it is taken that it did.
pub(super) fn ever_signed(config_dir: &Path, name: &str) -> bool {
    let ledger = ledger().lock().unwrap_or_else(|p| p.into_inner());
    ledger.unread.iter().any(|dir| dir == config_dir)
        || ledger
            .latest
            .contains_key(&(config_dir.to_path_buf(), name.to_string()))
}

/// The number of the latest save recorded for a tool, 0 when none is.
pub(super) fn recorded_seq(config_dir: &Path, name: &str) -> u64 {
    ledger()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .latest
        .get(&(config_dir.to_path_buf(), name.to_string()))
        .map_or(0, |l| l.seq)
}

/// Records a tool's latest signature and keeps the folder's record in the keychain.
pub(super) fn record_latest(config_dir: &Path, name: &str, signature: &str, seq: u64) {
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
pub(super) async fn keep_latest(config_dir: &Path, store: &crate::tools::auth::Secrets) {
    let _one_at_a_time = super::TRUST.writing.lock().await;
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
pub(super) async fn load_latest(config_dir: &Path, store: &crate::tools::auth::Secrets) {
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

pub(super) fn signing_key(config_dir: &Path) -> Option<[u8; 32]> {
    keys()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|(dir, _)| dir == config_dir)
        .map(|(_, key)| *key)
}

/// The signing key from the keychain, made the first time it is needed. Without a keychain,
/// tools are not signed and the hash pin alone holds them to what the user approved.
pub(in crate::chat::toolkit) async fn ensure_signing_key(config_dir: &Path) {
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
pub(in crate::chat::toolkit) async fn ensure_signing_key_in(
    config_dir: &Path,
    store: &crate::tools::auth::Secrets,
) {
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
pub(crate) fn unlock_signing(config_dir: &Path) {
    if dir(config_dir).is_dir() {
        tauri::async_runtime::block_on(ensure_signing_key(config_dir));
    }
}

/// HMAC-SHA256 (RFC 2104) of `message` under `key`, as hex.
pub(super) fn hmac(key: &[u8; 32], message: &[u8]) -> String {
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
pub(super) fn signed_part(tool: &ScriptTool) -> Vec<u8> {
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

pub(super) fn sign(config_dir: &Path, tool: &ScriptTool) -> Option<String> {
    let key = signing_key(config_dir)?;
    Some(hmac(&key, &signed_part(tool)))
}

/// Whether the app signed this state of a tool, at some time: for History entries and other
/// earlier states. True when there is no key to check with.
pub(super) fn signed_by_app(config_dir: &Path, tool: &ScriptTool) -> bool {
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
pub(super) fn verifies(config_dir: &Path, tool: &ScriptTool) -> bool {
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
