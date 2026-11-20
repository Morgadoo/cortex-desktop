mod connector;
mod dispatch;
mod handle;
mod policies;
pub use handle::ConnectorHandle;
use handle::ConnectorOperation;
#[cfg(any(test, feature = "test-utils"))]
pub mod testing;

pub use connector::{
    ApprovalDecision, ApprovalGate, Capabilities, Connector, ConnectorDescriptor,
    ConnectorManifest, Credentialed, RegistryHost, Session, ToolDefinition,
};

use super::error::{AuthError, ConnectError, ToolError};
use super::exposure::{ApprovalReason, Exposure, Gate, Scope};
use super::hidden_text;
use super::result_store::MAX_PREVIEW_BYTES;
use crate::credentials::{Activity, ConnectorStatus, Interactive, Readiness, Stored};
use crate::state::{ConnectorState, ToolPolicy};
use async_trait::async_trait;
use log::{error, info, warn};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

/// Who is making a call, for [`ToolRegistry::dispatch_with_transport`].
#[derive(Debug, Default, Clone, Copy)]
pub struct CallContext<'a> {
    /// The scheduled run making the call.
    pub run: Option<&'a str>,
    /// The conversation the call belongs to, if the web app named one.
    pub conversation: Option<&'a str>,
    /// The connectors whose tools the model was offered, where narrower than every enabled one.
    pub offered: Option<&'a [String]>,
    /// The caller keeps long results itself (the chat engine saves them to a file that lasts),
    /// so the result comes back whole instead of as a preview in the in-memory store.
    pub whole: bool,
}

pub struct ToolRegistry {
    /// Tool results too long for a reply, kept for `read_tool_result`.
    pub result_store: Arc<super::result_store::ResultStore>,
    connectors: RwLock<HashMap<String, Arc<ConnectorHandle>>>,
    state: RwLock<ConnectorState>,
    activity: RwLock<HashMap<String, Activity>>,
    operations: RwLock<HashMap<String, Arc<ConnectorOperation>>>,
    approval_gate: RwLock<Option<Arc<dyn ApprovalGate>>>,
    host: RwLock<Option<Arc<dyn RegistryHost>>>,
    exposure: std::sync::Mutex<super::exposure::Tracker>,
    /// Told about a failure that should never happen (no approval gate wired), for crash
    /// reporting; the app sets it with [`ToolRegistry::report_failures_to`].
    on_failure: RwLock<fn(&'static str)>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            result_store: Arc::new(super::result_store::ResultStore::default()),
            connectors: RwLock::new(HashMap::new()),
            state: RwLock::new(ConnectorState::default()),
            activity: RwLock::new(HashMap::new()),
            operations: RwLock::new(HashMap::new()),
            approval_gate: RwLock::new(None),
            host: RwLock::new(None),
            exposure: Default::default(),
            on_failure: RwLock::new(|message| log::error!("{message}")),
        }
    }

    /// Where failures that should never happen are reported (the app's crash reporting).
    pub fn report_failures_to(&self, report: fn(&'static str)) {
        *self
            .on_failure
            .write()
            .expect("failure reporter lock poisoned") = report;
    }

    pub fn set_approval_gate(&self, gate: Arc<dyn ApprovalGate>) {
        *self
            .approval_gate
            .write()
            .expect("approval gate lock poisoned") = Some(gate);
    }

    /// Shared by connection attempts and configuration changes, including replacement instances.
    fn connector_operation(&self, connector_id: &str) -> Arc<ConnectorOperation> {
        self.operations
            .write()
            .expect("operations lock poisoned")
            .entry(connector_id.to_string())
            .or_default()
            .clone()
    }

    pub fn cancel_connection(&self, connector_id: &str, attempt_id: &str) {
        let operation = self.connector_operation(connector_id);
        operation
            .cancelled_attempts
            .write()
            .expect("cancellation lock poisoned")
            .insert(attempt_id.to_string());
        operation.cancel.notify_waiters();
    }

    pub fn try_connector_operation(
        &self,
        connector_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, ConnectError> {
        self.connector_operation(connector_id).busy.clone().try_lock_owned().map_err(|_| {
            ConnectError::Failed(
                "A connection or configuration change is already in progress for this connector".into(),
            )
        })
    }

    pub fn set_host(&self, host: Arc<dyn RegistryHost>) {
        *self.host.write().expect("host lock poisoned") = Some(host);
    }

    fn host(&self) -> Option<Arc<dyn RegistryHost>> {
        self.host.read().expect("host lock poisoned").clone()
    }

    pub fn register<C: Connector + 'static>(&self, connector: C) {
        let id = connector.id().to_string();
        let mut map = self.connectors.write().expect("registry lock poisoned");
        map.insert(id, Arc::new(ConnectorHandle::new(connector)));
    }

    /// Evicts a connector and marks it retired, returning it so its session can be closed.
    #[must_use = "a connector with a live session must have it dropped; see ToolRegistry::retire"]
    fn unregister(&self, connector_id: &str) -> Option<Arc<ConnectorHandle>> {
        let evicted = self
            .connectors
            .write()
            .expect("registry lock poisoned")
            .remove(connector_id);
        if let Some(handle) = &evicted {
            handle.retire();
        }
        self.activity
            .write()
            .expect("activity lock poisoned")
            .remove(connector_id);
        evicted
    }

    /// Swaps in a connector built from configuration that has just changed.
    ///
    /// The outgoing one is retired and its session closed before the replacement is registered,
    /// so a call already in flight cannot reach the configuration it was built from.
    pub async fn replace<C: Connector + 'static>(&self, connector: C) {
        let id = connector.id().to_string();
        self.retire(&id).await;
        self.register(connector);
        if let Some(host) = self.host() {
            host.notify_changed(&id);
        }
    }

    /// Evicts a connector and closes whatever session it held.
    pub async fn retire(&self, connector_id: &str) {
        if let Some(handle) = self.unregister(connector_id) {
            if let Some(session) = handle.as_session() {
                session.drop_session().await;
            }
        }
    }

    /// Nothing is dispatchable once this returns, which is what prevents the outgoing account's
    /// connectors being reached while their sessions close.
    pub fn take_connectors(&self) -> Vec<Arc<ConnectorHandle>> {
        let mut map = self.connectors.write().expect("registry lock poisoned");
        self.activity
            .write()
            .expect("activity lock poisoned")
            .clear();
        map.drain()
            .map(|(_, handle)| {
                handle.retire();
                handle
            })
            .collect()
    }

    /// Snapshot of all registered connectors, for iteration outside the lock.
    pub fn connectors(&self) -> Vec<Arc<ConnectorHandle>> {
        self.connectors
            .read()
            .expect("registry lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    fn get(&self, connector_id: &str) -> Option<Arc<ConnectorHandle>> {
        self.connectors
            .read()
            .expect("registry lock poisoned")
            .get(connector_id)
            .cloned()
    }

    fn activity_of(&self, connector_id: &str) -> Activity {
        self.activity
            .read()
            .expect("activity lock poisoned")
            .get(connector_id)
            .cloned()
            .unwrap_or_default()
    }

    fn has_ever_succeeded(&self, connector_id: &str) -> bool {
        self.state
            .read()
            .expect("state lock poisoned")
            .succeeded_once
            .contains(connector_id)
    }

    fn record_success(&self, connector_id: &str) {
        self.set_activity(connector_id, Activity::Succeeded);
        let first_time = {
            let mut state = self.state.write().expect("state lock poisoned");
            state
                .succeeded_once
                .insert(connector_id.to_string())
                .then(|| state.clone())
        };
        // Only the first success carries information, so a run of calls writes nothing.
        if let Some(state) = first_time {
            self.persist(&state);
        }
    }

    /// `error` is shown in Settings; only `kind` is logged.
    fn record_failure(&self, connector_id: &str, kind: &str, error: String) {
        warn!("connector {connector_id} failed: {kind}");
        self.set_activity(connector_id, Activity::Failed { error });
    }

    pub fn forget_success_history(&self, connector_id: &str) {
        let cleared = {
            let mut state = self.state.write().expect("state lock poisoned");
            state.succeeded_once.remove(connector_id);
            state.clone()
        };
        self.persist(&cleared);
    }

    fn persist(&self, state: &ConnectorState) {
        if let Some(host) = self.host() {
            host.persist(state);
        }
    }

    fn set_activity(&self, connector_id: &str, activity: Activity) {
        let previous = self
            .activity
            .write()
            .expect("activity lock poisoned")
            .insert(connector_id.to_string(), activity.clone());
        if previous.as_ref() != Some(&activity) {
            if let Some(host) = self.host() {
                host.notify_changed(connector_id);
            }
        }
    }

    fn manifest_of(&self, connector: &Arc<ConnectorHandle>) -> ConnectorManifest {
        let descriptor = connector.descriptor();
        let capabilities = Capabilities {
            credentials: connector.as_credentialed().is_some(),
            session: connector.as_session().is_some(),
        };
        let readiness = connector.readiness();
        let activity = self.activity_of(&descriptor.id);
        let status = ConnectorStatus::derive(
            &readiness,
            capabilities.has_remote(),
            &activity,
            self.has_ever_succeeded(&descriptor.id),
        );
        let tools: Vec<ToolDefinition> = connector
            .tools()
            .into_iter()
            .map(|mut tool| {
                if let Some(policy) = self.saved_tool_policy(&descriptor.id, &tool.name) {
                    tool.policy = policy;
                }
                tool
            })
            .collect();
        let always_allowed = tools
            .iter()
            .filter(|tool| self.is_always_allowed(&descriptor.id, &tool.name))
            .map(|tool| tool.name.clone())
            .collect();

        ConnectorManifest {
            enabled: self.is_connector_enabled(&descriptor.id),
            activity: capabilities.has_remote().then_some(activity),
            capabilities,
            readiness,
            status,
            tools,
            always_allowed,
            id: descriptor.id,
            display_name: descriptor.display_name,
            description: descriptor.description,
            version: descriptor.version,
            icon: descriptor.icon,
            auth: descriptor.auth,
        }
    }

    /// Builtins are discovered independently of connector manifests and preferences.
    pub fn builtin_tools(&self) -> Vec<ToolDefinition> {
        if !self.result_store.is_active() {
            return vec![];
        }
        vec![super::result_reader::definition()]
    }

    pub fn all_manifests(&self) -> Vec<ConnectorManifest> {
        self.connectors()
            .iter()
            .map(|connector| self.manifest_of(connector))
            .collect()
    }

    #[allow(dead_code)]
    pub fn get_manifest(&self, connector_id: &str) -> Option<ConnectorManifest> {
        self.get(connector_id)
            .map(|connector| self.manifest_of(&connector))
    }

    /// Obtains credentials where the connector has any, then opens a session where it has one.
    ///
    /// `authenticate` is called unconditionally and relies on its own idempotence. A credential
    /// written by this call is withdrawn if the session then fails to open.
    pub async fn connect(
        &self,
        connector_id: &str,
        input: Option<Value>,
        mode: Interactive,
    ) -> Result<(), ConnectError> {
        self.connect_attempt(connector_id, input, mode, None).await
    }

    pub async fn connect_attempt(
        &self,
        connector_id: &str,
        input: Option<Value>,
        mode: Interactive,
        attempt_id: Option<&str>,
    ) -> Result<(), ConnectError> {
        let _operation = self.try_connector_operation(connector_id)?;
        let connector = self
            .get(connector_id)
            .ok_or_else(|| ConnectError::UnknownConnector(connector_id.into()))?;

        let credentialed = connector.as_credentialed();
        let session = connector.as_session();
        if credentialed.is_none() && session.is_none() {
            return Err(ConnectError::NothingToConnect(connector_id.into()));
        }

        let mut wrote = Stored::Nothing;
        let operation = self.connector_operation(connector_id);
        let mut cancelled = false;
        let outcome: Result<(), ConnectError> = tokio::select! {
            biased;
            _ = operation.cancelled(attempt_id) => {
                cancelled = true;
                Err(AuthError::Cancelled.into())
            }
            result = async {
                if let Some(credentialed) = credentialed {
                    wrote = credentialed.authenticate(input, mode).await?;
                }
                if let Some(session) = session {
                    session.ensure().await?;
                }
                Ok(())
            } => result,
        };

        match &outcome {
            Ok(()) => self.record_success(connector_id),
            // Startup cannot prompt for missing credentials; unfinished setup is not a failure.
            Err(ConnectError::Auth(AuthError::RequiresSetup)) if mode == Interactive::No => {}
            Err(e) => {
                if wrote == Stored::Something {
                    if let Some(credentialed) = credentialed {
                        let _ = credentialed.forget().await;
                    }
                }
                if cancelled {
                    self.set_activity(connector_id, Activity::Untried);
                } else {
                    self.record_failure(connector_id, e.kind(), e.to_string());
                }
            }
        }
        if let Some(attempt_id) = attempt_id {
            operation
                .cancelled_attempts
                .write()
                .map_err(|_| ConnectError::Failed("cancellation lock poisoned".into()))?
                .remove(attempt_id);
        }
        outcome
    }

    /// Drops any live session and erases the stored credentials.
    ///
    /// Available only where there are credentials; a session-only connector is switched off with
    /// the enable toggle.
    pub async fn disconnect(&self, connector_id: &str) -> Result<(), ConnectError> {
        let _operation = self.try_connector_operation(connector_id)?;
        self.disconnect_locked(connector_id).await
    }

    /// Caller holds the connector operation guard (also used by removal).
    pub async fn disconnect_locked(&self, connector_id: &str) -> Result<(), ConnectError> {
        let connector = self
            .get(connector_id)
            .ok_or_else(|| ConnectError::UnknownConnector(connector_id.into()))?;
        let credentialed = connector
            .as_credentialed()
            .ok_or_else(|| ConnectError::NothingToForget(connector_id.into()))?;

        if let Some(session) = connector.as_session() {
            session.drop_session().await;
        }
        credentialed.forget().await.map_err(ConnectError::Failed)?;
        self.activity
            .write()
            .expect("activity lock poisoned")
            .remove(connector_id);
        self.forget_success_history(connector_id);
        Ok(())
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
