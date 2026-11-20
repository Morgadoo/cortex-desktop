use crate::support::settings_file::SettingsFile;
use sentry::protocol::{DebugImage, Event, Exception, Frame, Mechanism};
use sentry::{ClientInitGuard, ClientOptions};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

const CONFIG_FAILURES: &[&str] = &[
    "Settings load failed",
    "Settings save failed",
    "Connector state load failed",
    "Connector state save failed",
    "MCP configuration load failed",
    "MCP configuration save failed",
    "Mail configuration load failed",
    "Mail configuration save failed",
];

const CREDENTIAL_FAILURES: &[&str] = &[
    "Credential decoding failed",
    "Credential serialization failed",
];

const APPROVAL_FAILURES: &[&str] = &[
    "Approval gate missing",
    "Approval request emission failed",
    "Approval resolution emission failed",
];

pub(super) fn config_failure(message: &'static str) {
    capture_failure("cortex.config", message);
}

pub(super) fn credential_failure(message: &'static str) {
    capture_failure("cortex.credentials", message);
}

pub(super) fn approval_failure(message: &'static str) {
    capture_failure("cortex.approvals", message);
}

pub(super) fn mcp_result_serialization_failure() {
    capture_failure("cortex.connectors", "MCP result serialization failed");
}

fn capture_failure(logger: &'static str, message: &'static str) {
    if !crate::e2e_testing_enabled() {
        sentry::capture_event(Event {
            logger: Some(logger.into()),
            message: Some(message.into()),
            ..Default::default()
        });
    }
}

/// What the app may send about itself (Settings → General → Privacy), kept in
/// `diagnostics.json` and read before the app starts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiagnosticsPrefs {
    /// Crash and error reports, in builds that include reporting. Off unless turned on.
    pub error_reports: bool,
    /// Look for a newer version when the app starts. Checking from the menu always works.
    pub update_checks: bool,
}

impl Default for DiagnosticsPrefs {
    fn default() -> Self {
        Self {
            error_reports: false,
            update_checks: true,
        }
    }
}

/// The app's config folder, found before Tauri runs (as Tauri finds it for this identifier).
pub fn early_config_dir() -> Option<std::path::PathBuf> {
    use crate::rename_migration::ID;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match std::env::consts::OS {
        "macos" => home.map(|h| h.join("Library/Application Support").join(ID)),
        "windows" => std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join(ID)),
        _ => std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|c| c.join(ID)),
    }
}

/// Plain, because it is read before the app starts and before encryption at rest is unlocked.
const PREFS_FILE: SettingsFile<DiagnosticsPrefs> = SettingsFile::plain("diagnostics.json");

pub fn load_prefs(config_dir: &std::path::Path) -> DiagnosticsPrefs {
    PREFS_FILE.load(config_dir)
}

pub fn save_prefs(config_dir: &std::path::Path, prefs: &DiagnosticsPrefs) -> Result<(), String> {
    PREFS_FILE.save(config_dir, prefs).map_err(String::from)
}

/// Whether this build was made with error reporting in it at all.
pub fn compiled_in() -> bool {
    match std::env::consts::OS {
        "macos" => option_env!("CORTEX_SENTRY_DSN_KEY_MACOS"),
        "windows" => option_env!("CORTEX_SENTRY_DSN_KEY_WINDOWS"),
        "linux" => option_env!("CORTEX_SENTRY_DSN_KEY_LINUX"),
        _ => None,
    }
    .is_some_and(|k| !k.trim().is_empty())
}

pub(super) fn init() -> Option<ClientInitGuard> {
    if crate::e2e_testing_enabled() {
        return None;
    }
    // Reports go out only when the user turned them on.
    if !early_config_dir().is_some_and(|dir| load_prefs(&dir).error_reports) {
        return None;
    }
    let key = match std::env::consts::OS {
        "macos" => option_env!("CORTEX_SENTRY_DSN_KEY_MACOS"),
        "windows" => option_env!("CORTEX_SENTRY_DSN_KEY_WINDOWS"),
        "linux" => option_env!("CORTEX_SENTRY_DSN_KEY_LINUX"),
        _ => None,
    }?
    .trim();
    let Some(dsn) = platform_dsn(key, std::env::consts::OS) else {
        if !key.is_empty() {
            eprintln!("[Cortex reporting] invalid Sentry DSN key; reporting disabled");
        }
        return None;
    };
    let options = match client_options(dsn) {
        Ok(options) => options,
        Err(_) => {
            eprintln!("[Cortex reporting] could not initialize transport; reporting disabled");
            return None;
        }
    };
    Some(sentry::init(options))
}

fn platform_dsn(key: &str, os: &str) -> Option<sentry::types::Dsn> {
    if key.is_empty() || !key.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let project = match os {
        "macos" => 99,
        "windows" => 101,
        "linux" => 100,
        _ => return None,
    };
    format!("https://{key}@lumo.proton.me/api/core/v4/reports/sentry/{project}")
        .parse()
        .ok()
}

struct ReportingTransportFactory(reqwest::Client);

impl sentry::TransportFactory for ReportingTransportFactory {
    fn create_transport_with_options(
        &self,
        options: sentry::TransportOptions,
    ) -> Arc<dyn sentry::Transport> {
        Arc::new(
            sentry::transports::ReqwestHttpTransportOptions::from(options)
                .with_client(self.0.clone())
                .build(),
        )
    }
}

fn client_options(dsn: sentry::types::Dsn) -> Result<ClientOptions, reqwest::Error> {
    let timeout = Duration::from_secs(10);
    // Proton's reporting endpoint requires a content type omitted by the SDK transport.
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/x-sentry-envelope"),
    );
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(timeout)
        .build()?;
    let mut options = ClientOptions::new()
        .release(concat!("cortex-desktop@", env!("CARGO_PKG_VERSION")))
        .environment(if cfg!(debug_assertions) {
            "development"
        } else {
            "production"
        })
        .send_default_pii(false)
        .max_breadcrumbs(0)
        .shutdown_timeout(timeout)
        .before_send(panic_event);
    options.dsn = Some(dsn);
    options.transport = Some(Arc::new(ReportingTransportFactory(http)));
    Ok(options)
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

fn panic_event(event: Event<'static>) -> Option<Event<'static>> {
    let component = match (event.logger.as_deref(), event.message.as_deref()) {
        (Some("cortex.connectors"), Some("MCP result serialization failed")) => {
            Some("native_connectors")
        }
        (Some("cortex.config"), Some(message)) if CONFIG_FAILURES.contains(&message) => {
            Some("native_config")
        }
        (Some("cortex.credentials"), Some(message)) if CREDENTIAL_FAILURES.contains(&message) => {
            Some("native_credentials")
        }
        (Some("cortex.approvals"), Some(message)) if APPROVAL_FAILURES.contains(&message) => {
            Some("native_approvals")
        }
        _ => None,
    };
    if let Some(component) = component {
        return Some(Event {
            event_id: event.event_id,
            timestamp: event.timestamp,
            message: event.message,
            logger: event.logger,
            release: event.release,
            environment: event.environment,
            sdk: event.sdk,
            tags: [
                ("component".into(), component.into()),
                ("os".into(), std::env::consts::OS.into()),
                ("arch".into(), std::env::consts::ARCH.into()),
            ]
            .into(),
            ..Default::default()
        });
    }
    let panic = event.exception.into_iter().find(|exception| {
        exception
            .mechanism
            .as_ref()
            .is_some_and(|mechanism| mechanism.ty == "panic")
    })?;
    let mut stacktrace = panic.stacktrace;
    if let Some(stacktrace) = &mut stacktrace {
        stacktrace.registers.clear();
        for frame in &mut stacktrace.frames {
            *frame = Frame {
                function: frame.function.take(),
                symbol: frame.symbol.take(),
                module: frame.module.take(),
                package: frame.package.as_deref().map(basename),
                filename: frame.filename.as_deref().map(basename),
                lineno: frame.lineno,
                colno: frame.colno,
                in_app: frame.in_app,
                image_addr: frame.image_addr,
                instruction_addr: frame.instruction_addr,
                symbol_addr: frame.symbol_addr,
                addr_mode: frame.addr_mode.take(),
                ..Default::default()
            };
        }
    }
    let mut debug_meta = event.debug_meta.into_owned();
    debug_meta.images.retain_mut(|image| match image {
        DebugImage::Apple(image) => {
            image.name = basename(&image.name);
            true
        }
        DebugImage::Symbolic(image) => {
            image.name = basename(&image.name);
            image.debug_file = image.debug_file.as_deref().map(basename);
            true
        }
        _ => false,
    });
    Some(Event {
        event_id: event.event_id,
        timestamp: event.timestamp,
        level: sentry::Level::Fatal,
        release: event.release,
        environment: event.environment,
        platform: event.platform,
        sdk: event.sdk,
        debug_meta: Cow::Owned(debug_meta),
        exception: vec![Exception {
            ty: "panic".into(),
            value: Some("Native Rust panic".into()),
            mechanism: Some(Mechanism {
                ty: "panic".into(),
                handled: Some(false),
                ..Default::default()
            }),
            stacktrace,
            ..Default::default()
        }]
        .into(),
        tags: [
            ("component".into(), "native".into()),
            ("os".into(), std::env::consts::OS.into()),
            ("arch".into(), std::env::consts::ARCH.into()),
        ]
        .into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentry::protocol::{Stacktrace, User};

    #[test]
    fn selects_proton_projects_and_rejects_invalid_keys() {
        for (os, project) in [("macos", 99), ("windows", 101), ("linux", 100)] {
            let dsn = platform_dsn("publicKey123", os).unwrap();
            assert_eq!(dsn.public_key(), "publicKey123");
            assert_eq!(dsn.host(), "lumo.proton.me");
            assert_eq!(dsn.project_id().to_string(), project.to_string());
            assert_eq!(
                dsn.envelope_api_url().as_str(),
                format!(
                    "https://lumo.proton.me/api/core/v4/reports/sentry/api/{project}/envelope/"
                )
            );
        }
        for key in ["", "key@other.host", "key:password", "key/123", "key\n"] {
            assert!(platform_dsn(key, "macos").is_none());
        }
    }

    #[test]
    fn panic_payload_and_scope_data_are_excluded_but_stack_identity_survives() {
        let event = Event {
            message: Some("secret tool input".into()),
            user: Some(User {
                email: Some("private@example.com".into()),
                ..Default::default()
            }),
            server_name: Some("private-host".into()),
            extra: [("token".into(), "secret".into())].into(),
            exception: vec![Exception {
                value: Some("panic containing a credential".into()),
                mechanism: Some(Mechanism {
                    ty: "panic".into(),
                    ..Default::default()
                }),
                stacktrace: Some(Stacktrace {
                    frames: vec![Frame {
                        function: Some("connector::invoke".into()),
                        filename: Some("C:\\Users\\private\\connector.rs".into()),
                        abs_path: Some("/Users/private/connector.rs".into()),
                        context_line: Some("private source".into()),
                        vars: [("input".into(), "secret".into())].into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }]
            .into(),
            ..Default::default()
        };
        let event = panic_event(event).unwrap();
        let json = serde_json::to_string(&event).unwrap();
        for secret in ["private", "credential", "secret", "token"] {
            assert!(!json.contains(secret), "leaked {secret}: {json}");
        }
        assert!(json.contains("connector::invoke"));
        assert!(json.contains("connector.rs"));
        assert!(json.contains("Native Rust panic"));
    }

    #[test]
    fn ordinary_errors_are_not_sent() {
        assert!(panic_event(Event {
            message: Some("tool failed".into()),
            ..Default::default()
        })
        .is_none());
    }

    #[test]
    fn configuration_reports_accept_only_fixed_messages_and_exclude_scope_data() {
        let event = Event {
            logger: Some("cortex.config".into()),
            message: Some("MCP configuration load failed".into()),
            user: Some(User {
                email: Some("private@example.com".into()),
                ..Default::default()
            }),
            extra: [("token".into(), "secret".into())].into(),
            tags: [("account".into(), "private-account".into())].into(),
            ..Default::default()
        };
        let filtered = panic_event(event.clone()).unwrap();
        assert_eq!(filtered.message, event.message);
        let json = serde_json::to_string(&filtered).unwrap();
        assert!(!json.contains("private"));
        assert!(!json.contains("secret"));
        let mut arbitrary = event;
        arbitrary.message = Some("raw error with secret file path".into());
        assert!(panic_event(arbitrary).is_none());
    }

    #[test]
    fn credential_reports_exclude_secrets_paths_and_account_data() {
        for message in CREDENTIAL_FAILURES {
            let event = Event {
                logger: Some("cortex.credentials".into()),
                message: Some((*message).into()),
                user: Some(User {
                    id: Some("private-account".into()),
                    ..Default::default()
                }),
                extra: [("secret".into(), "private-token".into())].into(),
                tags: [("path".into(), "/Users/private/secret".into())].into(),
                ..Default::default()
            };
            let filtered = panic_event(event).unwrap();
            assert_eq!(filtered.tags["component"], "native_credentials");
            assert_eq!(filtered.message.as_deref(), Some(*message));
            let json = serde_json::to_string(&filtered).unwrap();
            assert!(!json.contains("private"));
            assert!(!json.contains("secret"));
        }
        assert!(panic_event(Event {
            logger: Some("cortex.credentials".into()),
            message: Some("unexpected error containing private credential".into()),
            ..Default::default()
        })
        .is_none());
    }

    #[test]
    fn approval_reports_exclude_tool_inputs_and_request_identifiers() {
        for message in APPROVAL_FAILURES {
            let filtered = panic_event(Event {
                logger: Some("cortex.approvals".into()),
                message: Some((*message).into()),
                extra: [("input".into(), "private tool data".into())].into(),
                tags: [("request_id".into(), "private-request".into())].into(),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(filtered.message.as_deref(), Some(*message));
            assert_eq!(filtered.tags["component"], "native_approvals");
            let json = serde_json::to_string(&filtered).unwrap();
            assert!(!json.contains("private"));
            assert!(!json.contains("request_id"));
        }
        assert!(panic_event(Event {
            logger: Some("cortex.approvals".into()),
            message: Some("User denied approval".into()),
            ..Default::default()
        })
        .is_none());
    }
}
