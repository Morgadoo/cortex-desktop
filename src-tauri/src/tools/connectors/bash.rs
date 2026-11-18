use super::filesystem::sanitize_user_path;
use crate::account::AccountId;
use crate::settings::load_account_settings;
use crate::tools::auth::{AuthMechanism, Readiness};
use crate::tools::child_process::background_command;
#[cfg(not(windows))]
use crate::tools::command_path;
use crate::tools::connector_state::ToolPolicy;
use crate::tools::error::ToolError;
use crate::tools::icons::bundled_icon_data_uri;
use crate::tools::registry::{Connector, ConnectorDescriptor, ToolDefinition};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::RwLock;
use std::time::Duration;
use tauri::AppHandle;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

pub struct BashConnector {
    enabled: RwLock<bool>,
    allowed_roots: RwLock<Vec<PathBuf>>,
}

impl BashConnector {
    pub fn new(enabled: bool, allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            enabled: RwLock::new(enabled),
            allowed_roots: RwLock::new(allowed_roots),
        }
    }

    pub fn from_settings(app_handle: &AppHandle, account: &AccountId) -> Self {
        let (enabled, allowed_roots) = load_bash_settings(app_handle, account);
        Self::new(enabled, allowed_roots)
    }

    fn allowed_roots_for_prompt(&self) -> Vec<String> {
        self.allowed_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|path| display_root_path(path.as_path()))
            .collect()
    }

    fn resolve_workdir(&self, workdir: Option<&str>) -> Result<PathBuf, ToolError> {
        let roots = self
            .allowed_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if roots.is_empty() {
            return Err(ToolError::AccessDenied(
                "no allowed directories configured for shell commands".into(),
            ));
        }

        let candidate = match workdir {
            Some(path) => sanitize_user_path(path)?,
            None => roots[0].clone(),
        };

        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            roots[0].join(candidate)
        };

        let canonical = resolved
            .canonicalize()
            .map_err(|e| ToolError::AccessDenied(format!("invalid workdir: {e}")))?;

        if !canonical.is_dir() {
            return Err(ToolError::InvalidInput(format!(
                "workdir is not a directory: {}",
                canonical.display()
            )));
        }

        for root in roots.iter() {
            let canonical_root = root
                .canonicalize()
                .map_err(|e| ToolError::AccessDenied(format!("invalid root: {e}")))?;
            if canonical.starts_with(&canonical_root) {
                return Ok(canonical);
            }
        }

        Err(ToolError::AccessDenied(format!(
            "workdir outside allowed roots: {}",
            canonical.display()
        )))
    }
}

fn load_bash_settings(app_handle: &AppHandle, account: &AccountId) -> (bool, Vec<PathBuf>) {
    let settings = load_account_settings(app_handle, account).unwrap_or_default();
    let roots = settings
        .filesystem_allowed_roots
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    (settings.bash_enabled, roots)
}

fn display_root_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn format_allowed_roots_block(roots: &[String]) -> String {
    if roots.is_empty() {
        return "Configure allowed folders under Filesystem before enabling shell commands.".into();
    }
    let listed = roots
        .iter()
        .map(|root| format!("- {root}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Commands may only run with a working directory under:\n{listed}")
}

fn bash_description(roots: &[String]) -> String {
    format!(
        "Run a shell command on this device. {}\n\
         Use `workdir` to choose a starting directory under an allowed root; defaults to the first allowed root. \
         Returns exit code, stdout, and stderr.",
        format_allowed_roots_block(roots)
    )
}

fn truncate_output(text: String) -> (String, bool) {
    if text.len() <= MAX_OUTPUT_BYTES {
        return (text, false);
    }
    let truncated = text[..text.floor_char_boundary(MAX_OUTPUT_BYTES)].to_string();
    (truncated, true)
}

#[async_trait]
impl Connector for BashConnector {
    fn id(&self) -> &str {
        "bash"
    }

    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            id: "bash".into(),
            display_name: "Shell".into(),
            description: None,
            version: "1.0.0".into(),
            icon: bundled_icon_data_uri("bash"),
            auth: AuthMechanism::None,
        }
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        let roots = self.allowed_roots_for_prompt();
        vec![ToolDefinition {
            read_only: Some(false),
            open_world: None,
            policy: ToolPolicy::Ask,
            name: "run".into(),
            title: Some("Run Shell Command".into()),
            description: bash_description(&roots),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Optional timeout in milliseconds",
                        "minimum": 1,
                        "maximum": MAX_TIMEOUT_MS
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory under an allowed root. Defaults to the first allowed root."
                    }
                },
                "required": ["command"]
            }),
        }]
    }

    fn readiness(&self) -> Readiness {
        let enabled = *self
            .enabled
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let roots = self
            .allowed_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !enabled {
            Readiness::blocked("Enable shell access")
        } else if roots.is_empty() {
            Readiness::blocked("Add folders first")
        } else {
            Readiness::Ready
        }
    }

    async fn invoke(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        match tool_name {
            "run" => self.run_command(input).await,
            _ => Err(ToolError::UnknownTool(tool_name.into())),
        }
    }
}

impl BashConnector {
    async fn run_command(&self, input: Value) -> Result<Value, ToolError> {
        let script = input.get("script");
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();

        if command.is_empty() && script.is_none() {
            return Err(ToolError::InvalidInput("command must not be empty".into()));
        }

        let timeout_ms = input
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS);

        let workdir = input.get("workdir").and_then(Value::as_str);
        let cwd = self.resolve_workdir(workdir)?;

        let mut process = if let Some(script) = script {
            // The native skill adapter supplies argv, so skill arguments never become shell code.
            // This is still the same permission-gated `run` operation.
            let program = script
                .get("program")
                .and_then(Value::as_str)
                .filter(|program| {
                    matches!(
                        *program,
                        "python"
                            | "python3"
                            | "node"
                            | "sh"
                            | "bash"
                            | "zsh"
                            | "ruby"
                            | "perl"
                            | "pwsh"
                    )
                })
                .ok_or_else(|| ToolError::InvalidInput("Unsupported script interpreter".into()))?;
            let args = script
                .get("args")
                .and_then(Value::as_array)
                .ok_or_else(|| ToolError::InvalidInput("Script arguments are required".into()))?;
            let args = args
                .iter()
                .map(|arg| {
                    arg.as_str().ok_or_else(|| {
                        ToolError::InvalidInput("Script arguments must be strings".into())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut process = Command::new(program);
            process.args(args);
            process
        } else {
            #[cfg(windows)]
            let mut process = Command::new("cmd");
            #[cfg(windows)]
            process.args(["/C", command]);
            #[cfg(not(windows))]
            let mut process = Command::new("sh");
            #[cfg(not(windows))]
            process.args(["-c", command]);
            process
        };
        process.current_dir(&cwd);
        #[cfg(not(windows))]
        process.env("PATH", command_path::search_path().await);
        // Vault secrets granted to the chat's agent, as environment variables.
        process.envs(crate::chat::vault::current());

        // Match Command::output's stdio defaults while retaining the process wrappers.
        process
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = background_command(process)
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let mut stdout_pipe = child
            .stdout()
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("stdout pipe unavailable".into()))?;
        let mut stderr_pipe = child
            .stderr()
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("stderr pipe unavailable".into()))?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // Retain the wrapper so timeout and output errors can both stop the process tree.
        let result = timeout(Duration::from_millis(timeout_ms), async {
            tokio::try_join!(
                child.wait(),
                stdout_pipe.read_to_end(&mut stdout),
                stderr_pipe.read_to_end(&mut stderr),
            )
        })
        .await;
        let result = match result {
            Ok(Ok((status, _, _))) => Ok(status),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err(format!("command timed out after {timeout_ms}ms")),
        };
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                Box::into_pin(child.kill()).await.map_err(|e| {
                    ToolError::ExecutionFailed(format!("{error}; failed to stop process tree: {e}"))
                })?;
                return Err(ToolError::ExecutionFailed(error));
            }
        };

        let stdout = String::from_utf8_lossy(&stdout).into_owned();
        let stderr = String::from_utf8_lossy(&stderr).into_owned();
        let (stdout, stdout_truncated) = truncate_output(stdout);
        let (stderr, stderr_truncated) = truncate_output(stderr);

        Ok(json!({
            "command": if script.is_some() { "[bundled skill script]" } else { command },
            "workdir": cwd.display().to_string(),
            "exit_code": status.code(),
            "stdout": stdout,
            "stderr": stderr,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "timed_out": false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_stops_shell_and_descendant() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("run.sh"),
            "sleep 30 &\nprintf '%s %s' \"$$\" \"$!\" > pids\nwait\n",
        )
        .unwrap();
        let connector = BashConnector::new(true, vec![dir.path().to_path_buf()]);
        let error = connector
            .run_command(json!({ "command": "sh run.sh", "timeout": 500 }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("command timed out after 500ms"));
        let pids = std::fs::read_to_string(dir.path().join("pids"))
            .expect("shell and descendant must have started before timeout");
        let pids: Vec<_> = pids.split_whitespace().collect();
        assert_eq!(pids.len(), 2);
        let mut surviving = Vec::new();
        // An orphan can briefly remain a zombie until its new parent reaps it.
        for _ in 0..100 {
            surviving.clear();
            for pid in &pids {
                let output = std::process::Command::new("ps")
                    .args(["-p", pid, "-o", "stat="])
                    .output()
                    .unwrap();
                assert!(output.stderr.is_empty(), "ps failed: {output:?}");
                let state = String::from_utf8(output.stdout).unwrap();
                if !state.trim().is_empty() && !state.trim().starts_with('Z') {
                    surviving.push(*pid);
                }
            }
            if surviving.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Clean up even if a regression leaves a process alive.
        for pid in &surviving {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", pid])
                .status();
        }
        assert!(
            surviving.is_empty(),
            "processes survived timeout: {surviving:?}"
        );
    }

    #[test]
    fn a_network_share_is_never_a_workdir() {
        let dir = tempfile::tempdir().unwrap();
        let connector = BashConnector::new(true, vec![dir.path().to_path_buf()]);
        for workdir in [r"\\attacker.example\share", "//attacker.example/share"] {
            assert!(
                matches!(
                    connector.resolve_workdir(Some(workdir)),
                    Err(ToolError::AccessDenied(message)) if message.contains("network")
                ),
                "{workdir}"
            );
        }
    }

    #[tokio::test]
    async fn command_captures_output_and_exit_status() {
        let dir = tempfile::tempdir().unwrap();
        let connector = BashConnector::new(true, vec![dir.path().to_path_buf()]);
        let command = if cfg!(windows) {
            "echo stdout & echo stderr >&2 & exit 7"
        } else {
            "echo stdout; echo stderr >&2; exit 7"
        };
        let result = connector
            .run_command(json!({ "command": command }))
            .await
            .unwrap();
        assert_eq!(result["exit_code"], 7);
        assert!(result["stdout"].as_str().unwrap().contains("stdout"));
        assert!(result["stderr"].as_str().unwrap().contains("stderr"));
    }

    #[test]
    fn output_limit_does_not_split_unicode() {
        let text = "a".repeat(MAX_OUTPUT_BYTES - 1) + "🦀";
        let (output, truncated) = truncate_output(text);
        assert!(truncated);
        assert_eq!(output, "a".repeat(MAX_OUTPUT_BYTES - 1));
        assert_eq!(truncate_output("é🦀".into()), ("é🦀".into(), false));
    }
}
