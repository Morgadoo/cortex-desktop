//! The vault (OpenBot's per-agent vault access): named secrets, such as API tokens, kept in
//! the OS keychain and handed only to the agents allowed to use them.
//!
//! A secret reaches a run as an environment variable (`$GITHUB_TOKEN`) in shell commands,
//! `run_code` and the user's own tools; the model is told the names, never the values. Any value
//! that turns up in a tool's output is masked before the model or the chat sees it.
//!
//! A secret is used by the agents that list it, and by every chat and agent when it is marked
//! "everywhere". The list (names, notes, who may use them) is `chat/vault.json`; values live in
//! the keychain only.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::account::AccountId;
use crate::chat::LOCAL_ACCOUNT;
use crate::tools::auth::keychain_secrets;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Secret {
    /// The environment variable's name, like `GITHUB_TOKEN`.
    pub name: String,
    /// What it is for, shown to the model so it knows when to use it.
    pub description: String,
    /// Usable in every chat and by every agent, not only the agents that list it.
    pub everywhere: bool,
}

/// Names a secret may not take: they would change how programs run.
const RESERVED: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "LANG",
    "SHELL",
    "USER",
    "PWD",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "PYTHONPATH",
    "NODE_OPTIONS",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

tokio::task_local! {
    /// The secrets of the tool call running in this task, as (name, value).
    static ENV: Vec<(String, String)>;
}

fn path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("chat").join("vault.json")
}

fn keychain_name(name: &str) -> String {
    format!("vault:{name}")
}

pub fn list(config_dir: &Path) -> Vec<Secret> {
    crate::chat::at_rest::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write(config_dir: &Path, secrets: &[Secret]) -> Result<(), String> {
    let p = path(config_dir);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::chat::at_rest::write(
        p,
        serde_json::to_string_pretty(secrets).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// `GITHUB_TOKEN`-style: capitals, digits and underscores, not starting with a digit.
pub fn valid_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if !ok {
        return Err("Use capitals, digits and underscores for the name, like GITHUB_TOKEN.".into());
    }
    if RESERVED.contains(&name) || name.starts_with("CORTEX_") {
        return Err(format!(
            "{name} is used by the system; choose another name."
        ));
    }
    Ok(())
}

/// Adds or updates a secret; `value` replaces the stored one when given (required when new).
pub async fn save(
    config_dir: &Path,
    secret: Secret,
    value: Option<String>,
) -> Result<Vec<Secret>, String> {
    valid_name(&secret.name)?;
    let mut all = list(config_dir);
    let exists = all.iter().any(|s| s.name == secret.name);
    let value = value.filter(|v| !v.is_empty());
    if !exists && value.is_none() {
        return Err("Give the secret's value.".into());
    }
    if let Some(value) = value {
        crate::chat::secrets::local()?
            .store(&keychain_name(&secret.name), &value)
            .await?;
    }
    match all.iter_mut().find(|s| s.name == secret.name) {
        Some(s) => *s = secret,
        None => all.push(secret),
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    write(config_dir, &all)?;
    Ok(all)
}

pub async fn delete(config_dir: &Path, name: &str) -> Result<Vec<Secret>, String> {
    let mut all = list(config_dir);
    all.retain(|s| s.name != name);
    write(config_dir, &all)?;
    crate::chat::secrets::local()?
        .delete(&keychain_name(name))
        .await?;
    Ok(all)
}

/// The secrets a run may use: those the agent was given (`agent`, by name), and those marked
/// everywhere.
pub fn granted(all: &[Secret], agent: Option<&[String]>) -> Vec<Secret> {
    all.iter()
        .filter(|s| s.everywhere || agent.is_some_and(|names| names.contains(&s.name)))
        .cloned()
        .collect()
}

/// Names and values of the granted secrets, read from the keychain; a missing value is skipped.
pub async fn env_for(config_dir: &Path, agent: Option<&[String]>) -> Vec<(String, String)> {
    let granted = granted(&list(config_dir), agent);
    if granted.is_empty() {
        return Vec::new();
    }
    let Ok(account) = AccountId::parse(LOCAL_ACCOUNT) else {
        return Vec::new();
    };
    let store = keychain_secrets(account);
    let mut out = Vec::new();
    for s in granted {
        match store.load(&keychain_name(&s.name)).await {
            Ok(Some(v)) => out.push((s.name, v)),
            Ok(None) => log::warn!("vault: {} has no value in the keychain", s.name),
            Err(e) => log::warn!("vault: could not read {}: {e}", s.name),
        }
    }
    out
}

/// One secret's value, read from the keychain: for a tool the user granted that secret by name.
pub async fn value(config_dir: &Path, name: &str) -> Option<String> {
    list(config_dir).iter().find(|s| s.name == name)?;
    let store = keychain_secrets(AccountId::parse(LOCAL_ACCOUNT).ok()?);
    store.load(&keychain_name(name)).await.ok().flatten()
}

/// Runs `f` with `env` as the secrets of the current tool call.
pub async fn scope<F: std::future::Future>(env: Vec<(String, String)>, f: F) -> F::Output {
    ENV.scope(env, f).await
}

/// The secrets of the tool call running now, for the code that starts processes.
pub fn current() -> Vec<(String, String)> {
    ENV.try_with(Clone::clone).unwrap_or_default()
}

/// Hides every secret value in `text`; very short values (under 4 characters) are left, as
/// masking them would garble ordinary output.
pub fn mask(text: &str, env: &[(String, String)]) -> String {
    let mut out = text.to_string();
    let mut env: Vec<_> = env.iter().filter(|(_, v)| v.chars().count() >= 4).collect();
    // Longest first, so a value containing another is masked whole.
    env.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (name, value) in env {
        if out.contains(value.as_str()) {
            out = out.replace(value.as_str(), &format!("[secret {name}]"));
        }
    }
    out
}

/// The system-prompt lines naming the secrets this run may use.
pub fn prompt(granted: &[Secret]) -> Option<String> {
    if granted.is_empty() {
        return None;
    }
    let lines: Vec<String> = granted
        .iter()
        .map(|s| {
            if s.description.trim().is_empty() {
                format!("- ${}", s.name)
            } else {
                format!("- ${}: {}", s.name, s.description.trim())
            }
        })
        .collect();
    Some(format!(
        "\n\n## Secrets\nThese are set as environment variables in shell commands, `run_code` and the user's own tools. Refer to them by name (in a shell, `\"${{NAME}}\"`; in Python, `os.environ[\"NAME\"]`); never print, echo or ask for their values, and never write them into files or answers.\n{}",
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(name: &str, everywhere: bool) -> Secret {
        Secret {
            name: name.into(),
            description: format!("{name} for tests"),
            everywhere,
        }
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("GITHUB_TOKEN").is_ok());
        assert!(valid_name("API_KEY_2").is_ok());
        for bad in [
            "",
            "github_token",
            "2FA",
            "MY-KEY",
            "PATH",
            "LD_PRELOAD",
            "CORTEX_X",
            "A B",
        ] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn agents_get_only_their_secrets() {
        let all = vec![
            secret("A_TOKEN", false),
            secret("B_TOKEN", false),
            secret("SHARED", true),
        ];
        let agent = vec!["B_TOKEN".to_string()];
        let names = |v: Vec<Secret>| v.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(
            names(granted(&all, Some(agent.as_slice()))),
            ["B_TOKEN", "SHARED"]
        );
        assert_eq!(names(granted(&all, None)), ["SHARED"]);
        let line = prompt(&granted(&all, Some(agent.as_slice()))).unwrap();
        assert!(line.contains("$B_TOKEN: B_TOKEN for tests") && !line.contains("A_TOKEN"));
        assert!(prompt(&[]).is_none());
    }

    #[test]
    fn values_are_masked_in_output() {
        let env = vec![
            ("TOKEN".to_string(), "ghp_abcdef123456".to_string()),
            ("LONG".to_string(), "ghp_abcdef123456XYZ".to_string()),
            ("PIN".to_string(), "12".to_string()),
        ];
        let out = mask(
            "got ghp_abcdef123456XYZ and ghp_abcdef123456 at 12:00",
            &env,
        );
        assert_eq!(out, "got [secret LONG] and [secret TOKEN] at 12:00");
    }

    #[tokio::test]
    async fn the_scope_carries_the_secrets_to_process_code() {
        assert!(current().is_empty());
        let seen = scope(vec![("X_TOKEN".into(), "v".into())], async { current() }).await;
        assert_eq!(seen, vec![("X_TOKEN".to_string(), "v".to_string())]);
        assert!(current().is_empty());
    }
}
