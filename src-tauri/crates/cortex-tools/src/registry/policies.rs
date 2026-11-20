//! Which connectors are on, and each tool's policy: ask, allow or always allow.

use super::*;

impl ToolRegistry {
    pub fn set_connector_enabled(&self, connector_id: &str, enabled: bool) {
        let mut state = self.state.write().expect("state lock poisoned");
        if enabled {
            state.disabled_connectors.remove(connector_id);
        } else {
            state.disabled_connectors.insert(connector_id.to_string());
        }
    }

    pub fn is_connector_enabled(&self, connector_id: &str) -> bool {
        !self
            .state
            .read()
            .expect("state lock poisoned")
            .disabled_connectors
            .contains(connector_id)
    }

    /// Stores every choice explicitly, AutoApprove included: an absent entry means the tool's
    /// declared default, which may be Ask.
    pub fn set_tool_policy(&self, connector_id: &str, tool_name: &str, policy: ToolPolicy) {
        let pin = (policy == ToolPolicy::AutoApprove)
            .then(|| self.pinnable(connector_id, tool_name))
            .flatten();
        let mut state = self.state.write().expect("state lock poisoned");
        state
            .tool_policies
            .entry(connector_id.to_string())
            .or_default()
            .insert(tool_name.to_string(), policy);
        let pins = state
            .approved_tools
            .entry(connector_id.to_string())
            .or_default();
        match pin {
            Some(fingerprint) => pins.insert(tool_name.to_string(), fingerprint),
            None => pins.remove(tool_name),
        };
        if let Some(tools) = state.always_allowed.get_mut(connector_id) {
            tools.remove(tool_name);
        }
    }

    /// Auto-approves the tool and lets it run even after the conversation has read untrusted
    /// content. Choosing any policy afterwards withdraws this.
    pub fn always_allow(&self, connector_id: &str, tool_name: &str) {
        self.set_tool_policy(connector_id, tool_name, ToolPolicy::AutoApprove);
        let pin = self.pinnable(connector_id, tool_name).unwrap_or_default();
        self.state
            .write()
            .expect("state lock poisoned")
            .always_allowed
            .entry(connector_id.to_string())
            .or_default()
            .insert(tool_name.to_string(), pin);
    }

    /// Whether [`always_allow`](Self::always_allow) holds: chosen, and the tool unchanged since.
    pub fn is_always_allowed(&self, connector_id: &str, tool_name: &str) -> bool {
        let Some(pinned) = self
            .state
            .read()
            .expect("state lock poisoned")
            .always_allowed
            .get(connector_id)
            .and_then(|tools| tools.get(tool_name))
            .cloned()
        else {
            return false;
        };
        self.pinnable(connector_id, tool_name)
            .is_none_or(|current| current == pinned)
    }

    /// The fingerprint to pin an approval to: MCP tools only, whose servers can change them
    /// under the user's feet.
    pub(super) fn pinnable(&self, connector_id: &str, tool_name: &str) -> Option<String> {
        if !connector_id.starts_with("mcp-") {
            return None;
        }
        self.get(connector_id)?
            .tools()
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .map(|tool| tool.fingerprint())
    }

    /// The user's choice if they made one, else the policy the connector declares for the tool.
    /// A tool the connector doesn't list (not yet discovered, or an unreadable cache) asks.
    pub fn get_tool_policy(&self, connector_id: &str, tool_name: &str) -> ToolPolicy {
        self.saved_tool_policy(connector_id, tool_name)
            .unwrap_or_else(|| {
                self.get(connector_id)
                    .and_then(|connector| {
                        connector
                            .tools()
                            .into_iter()
                            .find(|tool| tool.name == tool_name)
                    })
                    .map(|tool| tool.policy)
                    .unwrap_or(ToolPolicy::Ask)
            })
    }

    /// An auto-approval of an MCP tool counts only while the tool is as it was when the user
    /// approved it. Approvals saved before pinning are pinned to the tool as first seen, and the
    /// pin is saved at once: re-pinning on every launch would carry the approval over to
    /// whatever the server says next.
    pub(super) fn saved_tool_policy(
        &self,
        connector_id: &str,
        tool_name: &str,
    ) -> Option<ToolPolicy> {
        let (saved, pinned) = {
            let state = self.state.read().expect("state lock poisoned");
            let saved = state
                .tool_policies
                .get(connector_id)
                .and_then(|tools| tools.get(tool_name))
                .copied()?;
            let pinned = state
                .approved_tools
                .get(connector_id)
                .and_then(|tools| tools.get(tool_name))
                .cloned();
            (saved, pinned)
        };
        if saved != ToolPolicy::AutoApprove {
            return Some(saved);
        }
        let Some(current) = self.pinnable(connector_id, tool_name) else {
            return Some(saved);
        };
        match pinned {
            Some(pinned) if pinned != current => None,
            Some(_) => Some(saved),
            None => {
                let pinned = {
                    let mut state = self.state.write().expect("state lock poisoned");
                    state
                        .approved_tools
                        .entry(connector_id.to_string())
                        .or_default()
                        .insert(tool_name.to_string(), current);
                    state.clone()
                };
                self.persist(&pinned);
                Some(saved)
            }
        }
    }

    pub fn clear_tool_policies(&self, connector_id: &str) {
        let mut state = self.state.write().expect("state lock poisoned");
        state.tool_policies.remove(connector_id);
        state.approved_tools.remove(connector_id);
        state.always_allowed.remove(connector_id);
    }

    pub fn set_state(&self, state: ConnectorState) {
        *self.state.write().expect("state lock poisoned") = state;
    }

    pub fn state_snapshot(&self) -> ConnectorState {
        self.state.read().expect("state lock poisoned").clone()
    }
}
