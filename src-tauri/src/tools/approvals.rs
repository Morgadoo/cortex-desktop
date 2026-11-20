use super::exposure::ApprovalReason;
use super::registry::{ApprovalDecision, ApprovalGate};
use crate::reporting::approval_failure;
use async_trait::async_trait;
use log::{info, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const APPROVAL_TIMEOUT_ENV: &str = "CORTEX_APPROVAL_TIMEOUT_MS";

fn parse_timeout_ms(raw: &str) -> Option<Duration> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
}

fn approval_timeout() -> Duration {
    static TIMEOUT: OnceLock<Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        if !crate::e2e_testing_enabled() {
            return DEFAULT_APPROVAL_TIMEOUT;
        }
        let Ok(raw) = std::env::var(APPROVAL_TIMEOUT_ENV) else {
            return DEFAULT_APPROVAL_TIMEOUT;
        };
        match parse_timeout_ms(&raw) {
            Some(timeout) => {
                info!("approval timeout set to {timeout:?}");
                timeout
            }
            None => {
                warn!(
                    "ignoring invalid {APPROVAL_TIMEOUT_ENV}={raw:?}, using {DEFAULT_APPROVAL_TIMEOUT:?}"
                );
                DEFAULT_APPROVAL_TIMEOUT
            }
        }
    })
}

/// Approval requests waiting for the user's answer, by request id. Held in Tauri's managed state;
/// switching accounts denies everything pending.
#[derive(Default)]
pub struct PendingApprovals {
    waiting: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
    sequence: AtomicU64,
}

impl PendingApprovals {
    fn waiting(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<ApprovalDecision>>> {
        self.waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A new request's id, and where its answer arrives.
    pub fn register(&self) -> (String, oneshot::Receiver<ApprovalDecision>) {
        let id = format!(
            "approval-{}",
            self.sequence.fetch_add(1, Ordering::SeqCst) + 1
        );
        let (tx, rx) = oneshot::channel();
        self.waiting().insert(id.clone(), tx);
        (id, rx)
    }

    /// The user's answer to request `request_id`; ignored when nothing waits for it.
    pub fn respond(&self, request_id: &str, decision: ApprovalDecision) {
        if let Some(tx) = self.waiting().remove(request_id) {
            let _ = tx.send(decision);
        }
    }

    /// Stops waiting for `request_id`: the waiter sees a denial.
    pub fn cancel(&self, request_id: &str) {
        self.waiting().remove(request_id);
    }

    /// Denies every request still waiting.
    pub fn cancel_all(&self) {
        let mut waiting = self.waiting();
        if waiting.is_empty() {
            return;
        }
        info!("denying {} pending approval(s)", waiting.len());
        waiting.clear();
    }
}

pub struct InteractiveApprovalGate {
    app_handle: AppHandle,
}

impl InteractiveApprovalGate {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }
}

#[async_trait]
impl ApprovalGate for InteractiveApprovalGate {
    async fn request(
        &self,
        connector_id: &str,
        tool_name: &str,
        input: &Value,
        reason: Option<&ApprovalReason>,
    ) -> ApprovalDecision {
        let approvals = self.app_handle.state::<PendingApprovals>();
        let (request_id, rx) = approvals.register();
        if self
            .app_handle
            .emit(
                "tool:approval_request",
                serde_json::json!({
                    "request_id": request_id,
                    "connector_id": connector_id,
                    "tool_name": tool_name,
                    "input": super::hidden_text::reveal(input),
                    "reason": reason,
                }),
            )
            .is_err()
        {
            approval_failure("Approval request emission failed");
        }

        match tokio::time::timeout(approval_timeout(), rx).await {
            Ok(Ok(decision)) => decision,
            _ => {
                approvals.cancel(&request_id);
                if self
                    .app_handle
                    .emit(
                        "tool:approval_resolved",
                        serde_json::json!({ "request_id": request_id, "approved": false }),
                    )
                    .is_err()
                {
                    approval_failure("Approval resolution emission failed");
                }
                ApprovalDecision::Deny
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respond_wakes_the_registered_waiter() {
        let approvals = PendingApprovals::default();
        let (request_id, rx) = approvals.register();
        approvals.respond(&request_id, ApprovalDecision::Always);
        assert_eq!(rx.blocking_recv(), Ok(ApprovalDecision::Always));
    }

    #[test]
    fn cancelling_all_denies_every_waiter() {
        let approvals = PendingApprovals::default();
        let (_, first_rx) = approvals.register();
        let (_, second_rx) = approvals.register();
        approvals.cancel_all();
        assert!(first_rx.blocking_recv().is_err());
        assert!(second_rx.blocking_recv().is_err());
        assert!(approvals.waiting().is_empty());
    }

    #[test]
    fn cancel_drops_the_registered_waiter() {
        let approvals = PendingApprovals::default();
        let (request_id, rx) = approvals.register();
        approvals.cancel(&request_id);
        assert!(rx.blocking_recv().is_err());
        approvals.respond(&request_id, ApprovalDecision::Once);
    }

    #[test]
    fn request_ids_are_unique() {
        let approvals = PendingApprovals::default();
        assert_ne!(approvals.register().0, approvals.register().0);
    }
}
