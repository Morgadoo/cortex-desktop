//! Calling a tool: admission, approval, exposure to the outside, retries and the result.

use super::*;

impl ToolRegistry {
    #[cfg(any(feature = "hosted", feature = "test-utils", test))]
    pub async fn dispatch(
        &self,
        connector_id: &str,
        tool_name: &str,
        input: Value,
    ) -> Result<Value, ToolError> {
        self.dispatch_with_approval(connector_id, tool_name, input, false)
            .await
    }

    /// The override is resolved from a registered schedule run, never a global policy mutation.
    #[cfg(any(feature = "hosted", feature = "test-utils", test))]
    pub async fn dispatch_with_approval(
        &self,
        connector_id: &str,
        tool_name: &str,
        input: Value,
        auto_approve: bool,
    ) -> Result<Value, ToolError> {
        self.dispatch_with_access(connector_id, tool_name, input, auto_approve, false)
            .await
    }

    /// Schedule access is enforced independently of global approval preferences.
    #[cfg(any(feature = "hosted", feature = "test-utils", test))]
    pub async fn dispatch_with_access(
        &self,
        connector_id: &str,
        tool_name: &str,
        input: Value,
        auto_approve: bool,
        read_only: bool,
    ) -> Result<Value, ToolError> {
        self.dispatch_with_transport(
            connector_id,
            tool_name,
            input,
            auto_approve,
            read_only,
            false,
            CallContext::default(),
        )
        .await
    }

    /// `context` decides where [`exposure`](super::exposure) is tracked and enforced.
    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_with_transport(
        &self,
        connector_id: &str,
        tool_name: &str,
        input: Value,
        auto_approve: bool,
        read_only: bool,
        file_transport: bool,
        context: CallContext<'_>,
    ) -> Result<Value, ToolError> {
        let exposure_scope = Scope::of(context.run, context.conversation);
        if connector_id == "tool_results" {
            let result = match tool_name {
                "read" => self.result_store.read(self.result_store.scope(), &input),
                "complete_file" => self
                    .result_store
                    .complete_file(self.result_store.scope(), &input),
                _ => Err(ToolError::UnknownTool(tool_name.into())),
            };
            return result.map(|value| {
                if file_transport {
                    serde_json::json!({"value": value, "file_transport": false})
                } else {
                    value
                }
            });
        }
        let connector = self
            .get(connector_id)
            .ok_or_else(|| ToolError::UnknownConnector(connector_id.into()))?;

        if read_only {
            connector.check_read_only(tool_name)?;
        }

        if !self.is_connector_enabled(connector_id) {
            return Err(ToolError::ConnectorDisabled);
        }

        if let Readiness::Blocked { reason } = connector.readiness() {
            return Err(ToolError::NotReady(reason));
        }

        let policy = self.get_tool_policy(connector_id, tool_name);
        let tool = connector
            .tools()
            .into_iter()
            .find(|tool| tool.name == tool_name);
        self.admit(
            connector_id,
            tool_name,
            tool.as_ref(),
            connector.vetted(),
            policy,
            &input,
            auto_approve,
            &context,
        )
        .await?;

        let scope = self.result_store.scope();
        let allow_file = connector_id == "filesystem"
            && tool_name == "fs_read"
            && input.get("format").and_then(Value::as_str) == Some("file");
        let result = self
            .invoke_once_retrying(&connector, tool_name, input, read_only)
            .await
            .map_err(|error| error.bounded_for_model(MAX_PREVIEW_BYTES))
            .map(|mut value| {
                if matches!(connector_id, "proton_mail" | "browser")
                    || connector_id.starts_with("mcp-")
                {
                    hidden_text::strip_smuggled(&mut value);
                }
                value
            });
        self.record_exposure(
            connector_id,
            tool_name,
            exposure_scope,
            result.as_ref().ok(),
        );
        let has_remote = connector.as_credentialed().is_some() || connector.as_session().is_some();
        match &result {
            _ if !has_remote => {}
            Ok(_) => self.record_success(connector_id),
            Err(e) if e.is_transport() => {
                self.record_failure(connector_id, e.kind(), e.to_string())
            }
            Err(_) => {}
        }
        // Epoch zero is an uninitialized standalone registry. Production initializes
        // the account scope before registering connectors. Preserve the captured
        // epoch across the await so late results from an outgoing account are rejected.
        if scope == 0 {
            return result.map(|value| {
                if file_transport {
                    serde_json::json!({"value": value, "file_transport": false})
                } else {
                    value
                }
            });
        }
        match result {
            Ok(value) if context.whole => Ok(value),
            Ok(value) if file_transport => {
                self.result_store
                    .preview_with_transport(scope, connector_id, value, allow_file)
            }
            Ok(value) => self.result_store.preview(scope, connector_id, value),
            Err(error) => Err(error),
        }
    }

    /// Re-establishes a session that turned out to be dead and retries the call once.
    ///
    /// Applies to a transport failure on a connector with a session; every other failure is
    /// returned as it came.
    pub(super) async fn invoke_once_retrying(
        &self,
        connector: &Arc<ConnectorHandle>,
        tool_name: &str,
        input: Value,
        read_only: bool,
    ) -> Result<Value, ToolError> {
        let invoke = |input| async move {
            if read_only {
                connector.invoke_read_only(tool_name, input).await
            } else {
                connector.invoke(tool_name, input).await
            }
        };
        let first = invoke(input.clone()).await;
        let Err(error) = &first else {
            return first;
        };
        if !error.is_transport() {
            return first;
        }
        let Some(session) = connector.as_session() else {
            return first;
        };

        session.drop_session().await;
        invoke(input).await
    }

    /// The checks every call passes before it runs: the per-tool policy, the untrusted-content
    /// gate, and the user's approval when either asks for it. `auto_approve` says the caller
    /// already has the user's go-ahead for the call, which settles the policy but not the gate.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn admit(
        &self,
        connector_id: &str,
        tool_name: &str,
        tool: Option<&ToolDefinition>,
        vetted: bool,
        policy: ToolPolicy,
        input: &Value,
        auto_approve: bool,
        context: &CallContext<'_>,
    ) -> Result<(), ToolError> {
        if policy == ToolPolicy::Disable {
            return Err(ToolError::ToolDisabled);
        }
        let scope = Scope::of(context.run, context.conversation);
        let exposure = self
            .exposure_of(scope)
            .with_unvetted_tools(self.offers_unvetted_tools(context.offered));
        let gate = self.exposure().gate(
            connector_id,
            tool_name,
            tool,
            vetted,
            input,
            scope,
            exposure,
        );
        let gate = if self.is_always_allowed(connector_id, tool_name) {
            gate.always_allowed()
        } else {
            gate
        };
        let (needs_approval, reason) = match gate {
            Gate::Refuse(why) => {
                info!("refused {connector_id}.{tool_name}: {why}");
                return Err(ToolError::AccessDenied(why.into()));
            }
            Gate::Ask(reason) => (true, Some(reason)),
            Gate::Allow => (policy == ToolPolicy::Ask && !auto_approve, None),
        };
        if needs_approval {
            match self
                .request_approval(connector_id, tool_name, input, reason.as_ref())
                .await
            {
                ApprovalDecision::Deny => return Err(ToolError::ApprovalDenied),
                ApprovalDecision::Once => {}
                ApprovalDecision::Always => {
                    self.remember_always(connector_id, tool_name, reason.as_ref())
                }
            }
        }
        Ok(())
    }

    /// Adds what a call's result put in front of the model to its run, session and conversation.
    /// Called for failed calls too: an error can carry the remote's text.
    pub(super) fn record_exposure(
        &self,
        connector_id: &str,
        tool_name: &str,
        scope: Scope,
        result: Option<&Value>,
    ) {
        let exposed = self
            .exposure()
            .record(connector_id, tool_name, scope, result);
        if let Scope::Conversation(id) = scope {
            self.expose_conversation(id, exposed);
        }
    }

    /// Runs a tool that lives outside the registry (an OpenAPI server's operation, one of the
    /// user's own tools, computer use) behind the same gate as a connector's: a policy the user
    /// saved for it, and the untrusted-content gate, which treats a connector it does not know
    /// as one that sends data out. With no saved policy nothing asks for the policy's sake; the
    /// caller's approval mode has already decided that. The outer error is the gate's refusal,
    /// the inner one the tool's own failure.
    pub async fn dispatch_external<F, Fut>(
        &self,
        connector_id: &str,
        tool: &ToolDefinition,
        input: Value,
        auto_approve: bool,
        context: CallContext<'_>,
        run: F,
    ) -> Result<Result<String, String>, ToolError>
    where
        F: FnOnce(Value) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        let policy = self
            .saved_tool_policy(connector_id, &tool.name)
            .unwrap_or(ToolPolicy::AutoApprove);
        self.admit(
            connector_id,
            &tool.name,
            Some(tool),
            false,
            policy,
            &input,
            auto_approve,
            &context,
        )
        .await?;
        let result = run(input).await;
        let shown = Value::String(match &result {
            Ok(text) | Err(text) => text.clone(),
        });
        self.record_exposure(
            connector_id,
            &tool.name,
            Scope::of(context.run, context.conversation),
            Some(&shown),
        );
        Ok(result)
    }

    /// Whether the model is offered tools from a server added by hand: among `offered`, or among
    /// every enabled connector when the caller doesn't say.
    pub(super) fn offers_unvetted_tools(&self, offered: Option<&[String]>) -> bool {
        let unvetted = |connector: &ConnectorHandle| {
            !connector.vetted() && self.is_connector_enabled(connector.id())
        };
        match offered {
            Some(ids) => ids
                .iter()
                .filter_map(|id| self.get(id))
                .any(|connector| unvetted(&connector)),
            None => self
                .connectors()
                .iter()
                .any(|connector| unvetted(connector)),
        }
    }

    pub(super) fn exposure(&self) -> std::sync::MutexGuard<'_, crate::exposure::Tracker> {
        self.exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Forgets what the outgoing account's tools returned. Conversations' exposure goes with the
    /// account's state.
    pub fn reset_exposure(&self) {
        *self.exposure() = Default::default();
    }

    pub(super) fn exposure_of(&self, scope: Scope) -> Exposure {
        match scope {
            Scope::Conversation(id) => self
                .state
                .read()
                .expect("state lock poisoned")
                .exposed_conversations
                .get(id)
                .copied()
                .unwrap_or_default(),
            other => self.exposure().exposure(other),
        }
    }

    /// Persists only when a conversation's exposure grows, which happens a few times at most.
    pub(super) fn expose_conversation(&self, id: &str, exposed: Exposure) {
        if exposed.is_empty() {
            return;
        }
        let changed = {
            let mut state = self.state.write().expect("state lock poisoned");
            let conversations = &mut state.exposed_conversations;
            let changed = match conversations.get_mut(id) {
                Some(exposure) => exposure.merge(exposed),
                None => {
                    let seq = conversations.values().map(|e| e.seq).max().unwrap_or(0) + 1;
                    if conversations.len() >= crate::exposure::MAX_TRACKED_CONVERSATIONS {
                        let oldest = conversations
                            .iter()
                            .min_by_key(|(_, exposure)| exposure.seq)
                            .map(|(id, _)| id.clone());
                        if let Some(oldest) = oldest {
                            conversations.remove(&oldest);
                        }
                    }
                    let mut first = exposed;
                    first.seq = seq;
                    conversations.insert(id.to_string(), first);
                    true
                }
            };
            changed.then(|| state.clone())
        };
        if let Some(state) = changed {
            self.persist(&state);
        }
    }

    /// Forgets what a conversation was exposed to, once it is gone for good (an eval case's): the
    /// saved state keeps only conversations that can still continue.
    pub fn forget_conversation(&self, id: &str) {
        let removed = {
            let mut state = self.state.write().expect("state lock poisoned");
            state
                .exposed_conversations
                .remove(id)
                .is_some()
                .then(|| state.clone())
        };
        if let Some(state) = removed {
            self.persist(&state);
        }
    }

    /// "Don't ask again" on an approval card: the tool stops asking for its own sake, as the
    /// Auto-approve setting does. It never becomes [`always_allow`](Self::always_allow)ed: the
    /// hosted web app can answer approval cards, so overriding untrusted content needs Settings.
    pub(super) fn remember_always(
        &self,
        connector_id: &str,
        tool_name: &str,
        reason: Option<&ApprovalReason>,
    ) {
        if reason.is_some_and(|reason| reason.kind == "hidden_text") {
            return;
        }
        self.set_tool_policy(connector_id, tool_name, ToolPolicy::AutoApprove);
        let state = self.state_snapshot();
        self.persist(&state);
        if let Some(host) = self.host() {
            host.notify_changed(connector_id);
        }
    }

    pub(super) async fn request_approval(
        &self,
        connector_id: &str,
        tool_name: &str,
        input: &Value,
        reason: Option<&ApprovalReason>,
    ) -> ApprovalDecision {
        let gate = self
            .approval_gate
            .read()
            .expect("approval gate lock poisoned")
            .clone();
        match gate {
            Some(gate) => gate.request(connector_id, tool_name, input, reason).await,
            None => {
                (self
                    .on_failure
                    .read()
                    .expect("failure reporter lock poisoned"))(
                    "Approval gate missing"
                );
                error!("no approval gate wired; denying 'ask' tool call");
                ApprovalDecision::Deny
            }
        }
    }
}
