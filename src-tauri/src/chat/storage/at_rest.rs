//! Encryption at rest, from the app's side: the core (sealing, reading, writing, converting the
//! folder) is `cortex_store::at_rest`, re-exported here; this module adds what needs the
//! keychain: keeping the key, turning encryption on and off, and unlocking at start.

use std::path::Path;

pub use cortex_store::at_rest::*;

fn secrets() -> Result<crate::tools::auth::Secrets, String> {
    Ok(crate::tools::auth::keychain_secrets(
        crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT)?,
    ))
}

/// A new key, kept in the keychain (read back to be sure it is there), then sealing.
pub async fn enable(config_dir: &Path) -> Result<usize, String> {
    if is_unlocked() {
        return Ok(0);
    }
    let key = new_key();
    let hex = key_hex(&key);
    let store = secrets()?;
    store.store(KEY_NAME, &hex).await?;
    if store.load(KEY_NAME).await?.as_deref() != Some(hex.as_str()) {
        return Err("The key could not be kept in the keychain, so nothing was encrypted.".into());
    }
    let dir = config_dir.to_path_buf();
    tokio::task::spawn_blocking(move || enable_with(&dir, key))
        .await
        .map_err(|e| e.to_string())?
}

pub async fn disable(config_dir: &Path) -> Result<usize, String> {
    let dir = config_dir.to_path_buf();
    let n = tokio::task::spawn_blocking(move || disable_files(&dir))
        .await
        .map_err(|e| e.to_string())??;
    secrets()?.delete(KEY_NAME).await?;
    Ok(n)
}

/// At start, before anything reads the folder: unlock with the keychain's key when on.
pub fn unlock_at_start() {
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        return;
    };
    if !marker(&config_dir).enabled {
        return;
    }
    // The keychain records withdrawn credentials in this folder; it is set once, to the same
    // folder the app sets it to later.
    crate::tools::auth::set_pending_deletes_dir(config_dir.clone());
    let key = tauri::async_runtime::block_on(async { secrets()?.load(KEY_NAME).await });
    if let Err(e) = &key {
        eprintln!("cortex: the encryption key could not be read from the keychain: {e}");
    }
    unlock(
        config_dir,
        key.ok().flatten().as_deref().and_then(parse_key),
    );
}
