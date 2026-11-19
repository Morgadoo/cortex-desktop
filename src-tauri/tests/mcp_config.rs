use cortex_lib::{CustomHeader, HttpAuth, McpServerConfig, McpTransport};
use std::collections::HashMap;

fn http_config() -> McpServerConfig {
    McpServerConfig {
        id: "slack".into(),
        display_name: "Slack".into(),
        description: None,
        transport: McpTransport::Http {
            url: "https://mcp.slack.com/mcp".into(),
            request_headers: HashMap::new(),
            auth: HttpAuth::Oauth {
                scopes: vec!["chat:write".into(), "users:read".into()],
                resource: Some("https://mcp.slack.com".into()),
                client_id: Some("11528367424691.11551831861717".into()),
                authorize_params: HashMap::from([("token_access_type".into(), "offline".into())]),
            },
        },
    }
}

fn http_token_config() -> McpServerConfig {
    McpServerConfig {
        id: "hibob".into(),
        display_name: "HiBob".into(),
        description: None,
        transport: McpTransport::Http {
            url: "https://api.hibob.com/mcp".into(),
            request_headers: HashMap::new(),
            auth: HttpAuth::Token {
                scheme: "Basic".into(),
            },
        },
    }
}

fn http_headers_config() -> McpServerConfig {
    McpServerConfig {
        id: "dual".into(),
        display_name: "Dual".into(),
        description: None,
        transport: McpTransport::Http {
            url: "https://mcp.example.com/mcp".into(),
            request_headers: HashMap::new(),
            auth: HttpAuth::Headers {
                headers: vec![
                    CustomHeader {
                        name: "Authorization".into(),
                        prefix: "Bearer ".into(),
                    },
                    CustomHeader {
                        name: "x-api-key".into(),
                        prefix: String::new(),
                    },
                ],
            },
        },
    }
}

fn stdio_config() -> McpServerConfig {
    McpServerConfig {
        id: "local".into(),
        display_name: "Local".into(),
        description: None,
        transport: McpTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "some-server".into()],
            env: HashMap::new(),
            secret_env: vec![],
        },
    }
}

fn grafana_http_config() -> McpServerConfig {
    McpServerConfig {
        id: "grafana".into(),
        display_name: "Grafana Cloud".into(),
        description: None,
        transport: McpTransport::Http {
            url: "https://mcp.grafana.com/mcp".into(),
            request_headers: HashMap::from([(
                "X-Grafana-URL".into(),
                "https://my-stack.grafana.net".into(),
            )]),
            auth: HttpAuth::Oauth {
                scopes: vec![],
                resource: None,
                client_id: None,
                authorize_params: HashMap::new(),
            },
        },
    }
}

fn round_trips(cfg: &McpServerConfig) {
    let json = serde_json::to_string(cfg).expect("serialize");
    let back: McpServerConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        serde_json::to_value(cfg).unwrap(),
        serde_json::to_value(&back).unwrap(),
    );
}

#[test]
fn http_config_round_trips() {
    round_trips(&http_config());
}

#[test]
fn http_token_config_round_trips() {
    round_trips(&http_token_config());
}

#[test]
fn stdio_config_round_trips() {
    round_trips(&stdio_config());
}

#[test]
fn http_headers_config_round_trips() {
    round_trips(&http_headers_config());
}

#[test]
fn grafana_http_config_round_trips() {
    round_trips(&grafana_http_config());
}

#[test]
fn grafana_http_config_persists_request_headers() {
    let v = serde_json::to_value(grafana_http_config()).unwrap();
    assert_eq!(
        v["request_headers"]["X-Grafana-URL"],
        "https://my-stack.grafana.net"
    );
}

#[test]
fn http_headers_config_serializes_with_nested_headers_auth() {
    let v = serde_json::to_value(http_headers_config()).unwrap();
    assert_eq!(v["transport"], "http");
    assert_eq!(v["auth"]["type"], "headers");
    assert_eq!(v["auth"]["headers"][0]["name"], "Authorization");
    assert_eq!(v["auth"]["headers"][0]["prefix"], "Bearer ");
    assert_eq!(v["auth"]["headers"][1]["name"], "x-api-key");
    assert!(v["auth"]["headers"][0].get("value").is_none());
}

#[test]
fn http_config_serializes_flat_with_transport_tag_and_nested_auth() {
    // The frontend and mcp_servers.json depend on the transport tag plus `url`
    // staying at the top level, while auth-specific fields live under a nested,
    // tagged `auth` object (design B — no illegal states).
    let v = serde_json::to_value(http_config()).unwrap();
    assert_eq!(v["transport"], "http");
    assert_eq!(v["url"], "https://mcp.slack.com/mcp");
    assert_eq!(v["auth"]["type"], "oauth");
    assert_eq!(v["auth"]["client_id"], "11528367424691.11551831861717");
    // Provider-specific offline-access params live under the nested auth object.
    assert_eq!(
        v["auth"]["authorize_params"]["token_access_type"],
        "offline"
    );
    // client_id must not leak back to the top level.
    assert!(v.get("client_id").is_none());
    assert!(v.get("command").is_none());
}

#[test]
fn http_token_config_serializes_with_nested_token_auth() {
    let v = serde_json::to_value(http_token_config()).unwrap();
    assert_eq!(v["transport"], "http");
    assert_eq!(v["url"], "https://api.hibob.com/mcp");
    assert_eq!(v["auth"]["type"], "token");
    assert_eq!(v["auth"]["scheme"], "Basic");
    // The token secret itself is never part of the persisted config.
    assert!(v["auth"].get("token").is_none());
}

#[test]
fn stdio_config_serializes_flat_with_transport_tag() {
    let v = serde_json::to_value(stdio_config()).unwrap();
    assert_eq!(v["transport"], "stdio");
    assert_eq!(v["command"], "npx");
    assert!(v.get("url").is_none());
}

#[test]
fn persisted_config_never_contains_a_secret() {
    // Security invariant: secrets (OAuth client secret, auth token) live only in
    // the keychain, never in the persisted config. There is no such field.
    for cfg in [http_config(), http_token_config(), http_headers_config()] {
        let json = serde_json::to_string(&cfg).unwrap().to_lowercase();
        assert!(
            !json.contains("secret") && !json.contains("\"token\":"),
            "persisted config must not contain a secret: {json}"
        );
    }
}
