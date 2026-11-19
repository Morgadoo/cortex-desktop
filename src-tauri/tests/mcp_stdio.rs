#![cfg(feature = "test-utils")]

use cortex_lib::{
    keychain_secrets, AccountId, Connector, CredentialState, McpConnector, McpServerConfig,
    McpTransport, NoToolCache, Readiness, Secrets, ToolError,
};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

const SERVER_BIN: &str = env!("CARGO_BIN_EXE_test-mcp-server");

fn test_server_config() -> McpServerConfig {
    McpServerConfig {
        id: "test".into(),
        display_name: "Test Server".into(),
        description: None,
        transport: McpTransport::Stdio {
            command: SERVER_BIN.into(),
            args: Vec::new(),
            env: HashMap::new(),
            secret_env: Vec::new(),
        },
    }
}

fn test_secrets() -> Secrets {
    keychain_secrets(AccountId::parse("test-account").expect("valid account id"))
}

fn connector() -> McpConnector {
    McpConnector::new(
        test_server_config(),
        test_secrets(),
        Box::new(NoToolCache),
        CredentialState::Present,
    )
}

async fn open_session(connector: &McpConnector) {
    connector
        .as_session()
        .expect("a stdio server has a session")
        .ensure()
        .await
        .expect("session should open");
}

async fn close_session(connector: &McpConnector) {
    connector
        .as_session()
        .expect("a stdio server has a session")
        .drop_session()
        .await;
}

fn tool_result_text(value: &serde_json::Value) -> String {
    value["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn lists_tools_after_connecting() {
    let connector = connector();
    open_session(&connector).await;

    let tools = connector.tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for expected in ["echo", "add", "slow", "boom"] {
        assert!(
            names.contains(&expected),
            "missing tool {expected} in {names:?}"
        );
    }

    let add = tools.iter().find(|t| t.name == "add").unwrap();
    assert!(
        add.input_schema.get("properties").is_some(),
        "add tool should expose an input schema"
    );

    close_session(&connector).await;
}

#[tokio::test]
async fn calls_tools_and_returns_results() {
    let connector = connector();

    let echoed = connector
        .invoke("echo", json!({ "text": "hello" }))
        .await
        .expect("echo call");
    assert_eq!(tool_result_text(&echoed), "hello");

    let sum = connector
        .invoke("add", json!({ "a": 2, "b": 3 }))
        .await
        .expect("add call");
    assert_eq!(tool_result_text(&sum), "5");

    close_session(&connector).await;
}

#[tokio::test]
async fn tool_failure_is_surfaced() {
    let connector = connector();

    match connector.invoke("boom", json!({})).await {
        Err(_) => {}
        Ok(value) => assert_eq!(
            value["isError"],
            json!(true),
            "boom should surface an error, got {value}"
        ),
    }

    close_session(&connector).await;
}

#[tokio::test]
async fn slow_call_times_out() {
    let connector = McpConnector::with_call_timeout(
        test_server_config(),
        test_secrets(),
        Duration::from_millis(200),
    );

    let result = connector.invoke("slow", json!({ "seconds": 5 })).await;

    assert!(
        matches!(result, Err(ToolError::ExecutionFailed(_))),
        "expected a timeout error, got {result:?}"
    );

    close_session(&connector).await;
}

/// A stdio server has nothing to log in to, so it stays ready whether or not a session is open.
/// Its tool list, which only a live session can discover, is what changes.
#[tokio::test]
async fn dropping_the_session_clears_the_tools_but_not_readiness() {
    let connector = connector();
    open_session(&connector).await;
    assert!(!connector.tools().is_empty());

    close_session(&connector).await;

    assert!(
        connector.tools().is_empty(),
        "the discovered tools belonged to that session"
    );
    assert_eq!(connector.readiness(), Readiness::Ready);
}

/// The settings flow is add -> connect -> disconnect -> connect. Dropping a session must not leave
/// the connector permanently unusable, which is what a latched `closed` flag would do.
#[tokio::test]
async fn a_session_can_be_reopened_after_being_dropped() {
    let connector = connector();
    open_session(&connector).await;
    close_session(&connector).await;

    open_session(&connector).await;

    assert!(
        !connector.tools().is_empty(),
        "tools should be rediscovered on reconnect"
    );
    let echoed = connector
        .invoke("echo", json!({ "text": "hello" }))
        .await
        .expect("echo call after reconnect");
    assert_eq!(tool_result_text(&echoed), "hello");

    close_session(&connector).await;
}
