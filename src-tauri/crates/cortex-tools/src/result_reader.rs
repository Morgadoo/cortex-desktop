//! Internal harness tools. These are model-visible, not user-managed connectors.
use super::registry::ToolDefinition;
use serde_json::json;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "tool_results__read".into(), title: None,
        description: "Read a bounded page of a previously completed tool's output using its result_id. This never reruns the original tool, including writes. Offsets are UTF-8 bytes; follow next_offset until null. References expire on logout, restart or cache eviction. If unavailable, narrow a read-only source query; never repeat a write to recover output.".into(),
        input_schema: json!({"type":"object","properties":{"result_id":{"type":"string"},"offset":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":513,"maximum":16000,"default":16000}},"required":["result_id"]}),
        policy: Default::default(),
        read_only: Some(true),
        open_world: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::error::ToolError;
    use super::*;
    use crate::credentials::{AuthMechanism, Readiness};
    use crate::registry::{Connector, ConnectorDescriptor, ToolRegistry};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    struct Source(Arc<AtomicUsize>);
    #[async_trait]
    impl Connector for Source {
        fn id(&self) -> &str {
            "jira"
        }
        fn descriptor(&self) -> ConnectorDescriptor {
            ConnectorDescriptor {
                id: "jira".into(),
                display_name: "Jira".into(),
                description: None,
                version: "test".into(),
                icon: None,
                auth: AuthMechanism::None,
            }
        }
        fn tools(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "update".into(),
                ..Default::default()
            }]
        }
        fn readiness(&self) -> Readiness {
            Readiness::Ready
        }
        async fn invoke(&self, _: &str, _: Value) -> Result<Value, ToolError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"status":"updated","output":"🦀".repeat(20_000)}))
        }
    }
    #[test]
    fn builtins_require_an_active_account_and_never_become_connector_manifests() {
        let registry = ToolRegistry::new();
        assert!(registry.builtin_tools().is_empty());
        registry.result_store.activate(true);
        assert_eq!(registry.builtin_tools()[0].name, "tool_results__read");
        assert!(registry.all_manifests().is_empty());
        registry.result_store.activate(false);
        assert!(registry.builtin_tools().is_empty());
    }

    #[tokio::test]
    async fn registry_bounds_results_and_reader_never_reexecutes_the_source() {
        let registry = ToolRegistry::new();
        registry.result_store.activate(true);
        assert_eq!(registry.builtin_tools()[0].name, "tool_results__read");
        assert!(
            registry
                .all_manifests()
                .iter()
                .all(|manifest| manifest.id != "tool_results"),
            "saved-result retrieval must not appear in connector UI manifests"
        );
        let calls = Arc::new(AtomicUsize::new(0));
        registry.register(Source(calls.clone()));
        let preview = registry
            .dispatch("jira", "update", json!({}))
            .await
            .unwrap();
        let text = preview.as_str().unwrap();
        assert!(text.len() <= 32_000);
        let id = text
            .split("result_id=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        let input = json!({"result_id":id});
        assert_eq!(
            registry
                .result_store
                .source_connector(registry.result_store.scope(), &input)
                .unwrap(),
            "jira"
        );
        let page = registry
            .dispatch_with_access("tool_results", "read", input.clone(), true, true)
            .await
            .unwrap();
        assert!(page.as_str().unwrap().len() <= 16_000);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A stale connector setting from an earlier version cannot disable a harness builtin.
        registry.set_connector_enabled("tool_results", false);
        assert!(registry
            .dispatch("tool_results", "read", input.clone())
            .await
            .is_ok());
        registry.result_store.activate(false);
        assert!(registry.builtin_tools().is_empty());
        assert!(registry
            .dispatch("tool_results", "read", input)
            .await
            .is_err());
    }
}
