//! Exposes Tauri commands for the main webview to connect to the local scheduler,
//! synchronize schedule timings, retrieve queued runs, and report execution statuses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{State, Webview};

use super::coordinator::{Operation, Response, ScheduleCoordinator};
use super::store::{QueuedRun, RunStatus};
use super::Schedule;
use crate::account::AccountId;

pub const PROTOCOL_VERSION: u32 = 1;

/// A reference to native, connection-scoped execution permissions. Contains no policy overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleToolContext {
    pub connection_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExecutionPolicy {
    pub connector_ids: Vec<String>,
    pub auto_approve: bool,
    pub read_only_connector_ids: Vec<String>,
}

/// Native-derived access for the requested connector; never supplied per invocation by web.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolAuthorization {
    pub auto_approve: bool,
    pub read_only: bool,
    /// The connectors the scheduled task offers the model.
    pub connector_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub connection_id: String,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkAvailable {
    pub connection_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRun {
    run_id: String,
    schedule_id: String,
    timing_revision: u64,
    scheduled_for: DateTime<Utc>,
    manual: bool,
    late: bool,
    status: RunStatus,
    updated_at: DateTime<Utc>,
}

impl From<QueuedRun> for BridgeRun {
    fn from(run: QueuedRun) -> Self {
        Self {
            run_id: run.run_id,
            schedule_id: run.occurrence.schedule_id,
            timing_revision: run.occurrence.timing_revision,
            scheduled_for: run.occurrence.scheduled_for,
            manual: run.manual,
            late: run.late,
            status: run.status,
            updated_at: run.updated_at,
        }
    }
}

fn require_main(label: &str) -> Result<(), String> {
    crate::main_window::require(label, "schedule commands require the main webview")
}

#[tauri::command]
pub async fn schedule_connect(
    account_id: String,
    protocol_version: u32,
    webview: Webview,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<Connection, String> {
    require_main(webview.label())?;
    let account = AccountId::parse(&account_id)?;
    match coordinator
        .request(Operation::Connect {
            account,
            protocol_version,
        })
        .await?
    {
        Response::Connected(connection) => Ok(connection),
        _ => Err("unexpected scheduler response".into()),
    }
}

#[tauri::command]
pub async fn schedule_replace(
    connection_id: String,
    schedules: Vec<Schedule>,
    webview: Webview,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<(), String> {
    require_main(webview.label())?;
    match coordinator
        .request(Operation::Replace {
            connection_id,
            schedules,
        })
        .await?
    {
        Response::Done => Ok(()),
        _ => Err("unexpected scheduler response".into()),
    }
}

#[tauri::command]
pub async fn schedule_run_now(
    connection_id: String,
    schedule_id: String,
    webview: Webview,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<BridgeRun, String> {
    require_main(webview.label())?;
    match coordinator
        .request(Operation::RunNow {
            connection_id,
            schedule_id,
        })
        .await?
    {
        Response::Queued(run) => Ok(run.into()),
        _ => Err("unexpected scheduler response".into()),
    }
}

async fn runs(
    coordinator: &ScheduleCoordinator,
    operation: Operation,
) -> Result<Vec<BridgeRun>, String> {
    match coordinator.request(operation).await? {
        Response::Runs(runs) => Ok(runs.into_iter().map(BridgeRun::from).collect()),
        _ => Err("unexpected scheduler response".into()),
    }
}

#[tauri::command]
pub async fn schedule_pending(
    connection_id: String,
    webview: Webview,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<Vec<BridgeRun>, String> {
    require_main(webview.label())?;
    runs(&coordinator, Operation::Pending { connection_id }).await
}

#[tauri::command]
pub async fn schedule_unsettled(
    connection_id: String,
    webview: Webview,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<Vec<BridgeRun>, String> {
    require_main(webview.label())?;
    runs(&coordinator, Operation::Unsettled { connection_id }).await
}

#[tauri::command]
pub async fn schedule_report_status(
    connection_id: String,
    run_id: String,
    status: RunStatus,
    webview: Webview,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<(), String> {
    require_main(webview.label())?;
    match coordinator
        .request(Operation::Report {
            connection_id,
            run_id,
            status,
        })
        .await?
    {
        Response::Done => Ok(()),
        _ => Err("unexpected scheduler response".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_webview_may_use_the_bridge() {
        assert!(require_main("cortex-window").is_ok());
        for label in ["settings-overlay", "search", "login", ""] {
            assert!(require_main(label).is_err());
        }
    }

    #[test]
    fn run_dto_uses_the_public_flat_camel_case_contract() {
        let at = "2026-10-15T09:00:00Z".parse().unwrap();
        for (manual, late) in [(true, false), (false, false), (false, true)] {
            let dto = BridgeRun::from(QueuedRun {
                run_id: "run".into(),
                occurrence: super::super::Occurrence {
                    schedule_id: "schedule".into(),
                    timing_revision: 1,
                    scheduled_for: at,
                },
                manual,
                late,
                status: RunStatus::Pending,
                updated_at: at,
            });
            assert_eq!(
                serde_json::to_value(dto).unwrap(),
                serde_json::json!({
                    "runId": "run", "scheduleId": "schedule", "timingRevision": 1,
                    "manual": manual, "late": late,
                    "scheduledFor": "2026-10-15T09:00:00Z", "status": "pending", "updatedAt": "2026-10-15T09:00:00Z"
                })
            );
        }
    }
}
