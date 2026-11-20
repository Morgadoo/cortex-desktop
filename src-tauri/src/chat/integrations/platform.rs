//! What the chat platforms (Slack, Discord, Telegram) share: one connection at a time, kept up
//! while the platform is turned on, and who may talk to the assistant through it.
//!
//! Each platform keeps what is its own: the wire protocol, the shape of its messages, and its
//! saved settings (whose formats are users' files and differ between platforms).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::AppHandle;

/// Allows one connection loop per platform. A platform is one account's bot, so the process holds
/// at most one connection to it; starting again while one runs does nothing.
pub struct OneAtATime(AtomicBool);

impl OneAtATime {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// The right to run, or `None` while a loop already runs. Released when the claim drops.
    pub fn claim(&'static self) -> Option<Claim> {
        (!self.0.swap(true, Ordering::SeqCst)).then_some(Claim(&self.0))
    }
}

pub struct Claim(&'static AtomicBool);

impl Drop for Claim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A platform reached through a long-lived connection that it closes now and then.
#[async_trait::async_trait]
pub trait Connection: 'static {
    /// The name in the log.
    const NAME: &'static str;
    type Credentials: Send + Sync;

    /// Whether the user has turned it on.
    fn enabled(config_dir: &Path) -> bool;
    /// The saved tokens, or `None` when there are none (the loop then stops).
    async fn credentials() -> Option<Self::Credentials>;
    /// One connection, until the platform closes it or it fails.
    async fn session(
        app: &AppHandle,
        config_dir: &Path,
        credentials: &Self::Credentials,
    ) -> Result<(), String>;
}

/// Connects while the platform is on, reconnecting after a pause whenever a session ends.
pub(in crate::chat::integrations) fn keep_connected<P: Connection>(
    app: &AppHandle,
    running: &'static OneAtATime,
) {
    let Some(claim) = running.claim() else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _claim = claim;
        while let Ok(config_dir) = crate::settings::get_config_dir(&app) {
            if !P::enabled(&config_dir) {
                break;
            }
            let Some(credentials) = P::credentials().await else {
                break;
            };
            if let Err(e) = P::session(&app, &config_dir, &credentials).await {
                log::warn!("{}: {e}", P::NAME);
            }
            // Platforms ask clients to reconnect from time to time; a failure waits a little first.
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

/// Who wrote a message, as far as answering goes.
#[derive(Debug, PartialEq)]
pub enum Gate<'a> {
    /// The pairing command with the right code: pair the sender.
    Pair,
    /// The pairing command with a wrong code, or while pairing is closed.
    WrongCode,
    /// Someone who is not paired.
    Stranger,
    /// A paired user, and what they wrote.
    Owner(&'a str),
}

/// Sorts a message: `pair_command` followed by `pairing_code` pairs the sender; only paired users
/// get answers.
pub(in crate::chat::integrations) fn gate<'a, U: PartialEq>(
    text: &'a str,
    pair_command: &str,
    pairing_code: Option<&str>,
    paired: &[U],
    user: &U,
) -> Gate<'a> {
    if let Some(code) = text.strip_prefix(pair_command).map(str::trim) {
        return match pairing_code {
            Some(expected) if expected == code => Gate::Pair,
            _ => Gate::WrongCode,
        };
    }
    if !paired.contains(user) {
        return Gate::Stranger;
    }
    Gate::Owner(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_code_pairs_and_only_paired_users_are_answered() {
        let paired = ["u1".to_string()];
        let me = "u1".to_string();
        let other = "u2".to_string();
        assert_eq!(
            gate("pair 123", "pair", Some("123"), &paired, &other),
            Gate::Pair
        );
        assert_eq!(
            gate("pair 999", "pair", Some("123"), &paired, &other),
            Gate::WrongCode
        );
        assert_eq!(
            gate("pair 123", "pair", None, &paired, &other),
            Gate::WrongCode
        );
        assert_eq!(
            gate("hello", "pair", Some("123"), &paired, &other),
            Gate::Stranger
        );
        assert_eq!(
            gate("hello", "pair", None, &paired, &me),
            Gate::Owner("hello")
        );
        assert_eq!(
            gate("/pair 7", "/pair", Some("7"), &[1_i64], &2),
            Gate::Pair
        );
    }

    #[test]
    fn one_loop_at_a_time() {
        static RUNNING: OneAtATime = OneAtATime::new();
        let first = RUNNING.claim();
        assert!(first.is_some());
        assert!(RUNNING.claim().is_none());
        drop(first);
        assert!(RUNNING.claim().is_some());
    }
}
