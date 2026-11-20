//! The agent workbench: `run_code` in a rootless Podman container instead of bubblewrap on the
//! machine itself, where Podman is available (Linux).
//!
//! Bubblewrap runs code with the system's own programs, so an agent can use only what the user
//! installed (on an immutable system, next to nothing), and nothing it needs can be added. The
//! workbench is an image built on this computer from `assets/workbench/Containerfile` (Python with
//! its data libraries, Node.js, media tools, Blender), and every run is a fresh container that sees
//! only two folders: the conversation's workspace and, for an agent, its own home, which keeps the
//! packages it installs. The network is off unless the user allowed it for that agent; the user's
//! files, keychain, session bus and other containers are out of reach; files come back owned by
//! the user (`--userns=keep-id`). The NVIDIA GPU is passed in through CDI where the host has a
//! working spec, so Blender can render on it.
//!
//! Inside a toolbox or distrobox (Cortex's own development setup), Podman runs on the host through
//! `flatpak-spawn --host` or `distrobox-host-exec`; paths under the home folder are the same there.
//!
//! The image is built only when the user asks (Settings → Permissions → Agent workbench): it is
//! a large download. Until then, and wherever Podman is missing, `run_code` uses bubblewrap as
//! before.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::chat::store::{chat_dir, write_json};

pub const CONTAINERFILE: &str = include_str!("../../../assets/workbench/Containerfile");

/// The image this version of the app runs: a hash of its Containerfile, so a changed recipe
/// builds a new image instead of reusing a stale one.
pub fn image_tag() -> String {
    let hash = Sha256::digest(CONTAINERFILE.as_bytes());
    let hex: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("localhost/cortex-workbench:{hex}")
}

/// What the workbench can do now.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Podman cannot be reached; `run_code` uses bubblewrap.
    Unavailable {
        reason: String,
    },
    /// Podman works; the image is not built yet.
    Missing,
    Building,
    Ready {
        gpu: bool,
    },
    Failed {
        error: String,
    },
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);

fn cached() -> Option<Status> {
    STATUS.lock().ok().and_then(|s| s.clone())
}

fn remember(status: &Status) {
    if let Ok(mut s) = STATUS.lock() {
        *s = Some(status.clone());
    }
}

fn settings_path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("workbench.json")
}

/// Whether code runs in the workbench when it is ready (on unless the user turned it off).
pub fn enabled(config_dir: &Path) -> bool {
    crate::chat::at_rest::read_to_string(settings_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

pub fn set_enabled(config_dir: &Path, on: bool) -> Result<(), String> {
    write_json(
        &settings_path(config_dir),
        &serde_json::json!({ "enabled": on }),
    )
    .map_err(String::from)
}

/// How to start Podman: itself, or on the host from inside a toolbox or distrobox container.
fn podman_prefix() -> Option<&'static [OsString]> {
    static PREFIX: OnceLock<Option<Vec<OsString>>> = OnceLock::new();
    PREFIX
        .get_or_init(|| {
            if !cfg!(target_os = "linux") {
                return None;
            }
            let mut candidates: Vec<Vec<OsString>> = Vec::new();
            if let Some(podman) = crate::chat::sandbox::find_program("podman") {
                candidates.push(vec![podman.into_os_string()]);
            }
            let in_container =
                Path::new("/run/.containerenv").exists() || Path::new("/run/.toolboxenv").exists();
            if in_container {
                if let Some(spawn) = crate::chat::sandbox::find_program("flatpak-spawn") {
                    candidates.push(vec![
                        spawn.into_os_string(),
                        "--host".into(),
                        "podman".into(),
                    ]);
                }
                if let Some(exec) = crate::chat::sandbox::find_program("distrobox-host-exec") {
                    candidates.push(vec![exec.into_os_string(), "podman".into()]);
                }
            }
            candidates.into_iter().find(|prefix| {
                std::process::Command::new(&prefix[0])
                    .args(&prefix[1..])
                    .args(["version", "--format", "{{.Client.Version}}"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            })
        })
        .as_deref()
}

/// A Podman command, or None where Podman cannot be reached.
fn podman() -> Option<tokio::process::Command> {
    let prefix = podman_prefix()?;
    let mut c = tokio::process::Command::new(&prefix[0]);
    c.args(&prefix[1..]).stdin(Stdio::null());
    Some(c)
}

async fn quiet(mut command: tokio::process::Command) -> bool {
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Whether a container from the image can use the NVIDIA GPU (a working CDI spec on the host).
async fn gpu_works(tag: &str) -> bool {
    let Some(mut c) = podman() else { return false };
    c.args([
        "run",
        "--rm",
        "--network",
        "none",
        "--security-opt",
        "label=disable",
        "--device",
        "nvidia.com/gpu=0",
        tag,
        "true",
    ]);
    quiet(c).await
}

/// The workbench's state, checked once and then kept until a build changes it.
pub async fn status() -> Status {
    if let Some(s) = cached() {
        return s;
    }
    let status = if podman_prefix().is_none() {
        Status::Unavailable {
            reason: "Podman is not installed or cannot be reached.".into(),
        }
    } else {
        let tag = image_tag();
        let mut exists = podman().expect("podman reachable");
        exists.args(["image", "exists", &tag]);
        if quiet(exists).await {
            Status::Ready {
                gpu: gpu_works(&tag).await,
            }
        } else {
            Status::Missing
        }
    };
    remember(&status);
    status
}

/// The status as last checked, without waiting: what the `run_code` description goes by.
pub fn status_now() -> Option<Status> {
    cached()
}

/// Whether runs go to the workbench now: on, and built. Gives whether the GPU works.
pub async fn ready(config_dir: &Path) -> Option<bool> {
    if !enabled(config_dir) {
        return None;
    }
    match status().await {
        Status::Ready { gpu } => Some(gpu),
        _ => None,
    }
}

/// Builds the image, writing the build's output to `chat/workbench/build.log`.
pub async fn build(config_dir: &Path) -> Status {
    match status().await {
        Status::Unavailable { .. } | Status::Building => return status().await,
        _ => {}
    }
    remember(&Status::Building);
    let result = async {
        let dir = chat_dir(config_dir).join("workbench");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("Containerfile");
        std::fs::write(&file, CONTAINERFILE).map_err(|e| e.to_string())?;
        let log_path = dir.join("build.log");
        let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
        let err_log = log.try_clone().map_err(|e| e.to_string())?;
        let tag = image_tag();
        let mut c = podman().ok_or("Podman cannot be reached.")?;
        c.arg("build")
            .arg("--pull=missing")
            .arg("--tag")
            .arg(&tag)
            .arg("--file")
            .arg(&file)
            .arg(&dir)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err_log));
        let ok = c.status().await.is_ok_and(|s| s.success());
        if ok {
            Ok(tag)
        } else {
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            let last: Vec<&str> = text.lines().rev().take(12).collect();
            Err(last.into_iter().rev().collect::<Vec<_>>().join("\n"))
        }
    }
    .await;
    let status = match result {
        Ok(tag) => Status::Ready {
            gpu: gpu_works(&tag).await,
        },
        Err(error) => Status::Failed { error },
    };
    remember(&status);
    status
}

/// One run in the workbench.
pub struct Run<'a> {
    /// The conversation's workspace: the working directory, and the only folder besides the
    /// agent's home that the run can write.
    pub workspace: &'a Path,
    pub agent_home: Option<&'a Path>,
    pub network: bool,
    pub gpu: bool,
    /// `python3`, `node` or `bash`, from the image.
    pub interpreter: &'a str,
    pub script: &'a Path,
    pub timeout_seconds: u64,
    /// `KEY=value` lines for the run's environment.
    pub env_file: &'a Path,
    /// The container's name, to stop it if the run outlives its time.
    pub name: &'a str,
}

/// The arguments after `podman`: a fresh container that sees only the workspace and the agent's
/// home, and stops itself when its time is up.
pub fn run_args(run: &Run<'_>, tag: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "run",
        "--rm",
        // A small init as the first process: `timeout` cannot run as process 1 (it needs a
        // process group of its own), and the init also reaps what the program leaves behind.
        "--init",
        "--interactive=false",
        "--userns=keep-id",
        "--security-opt",
        "label=disable",
        "--security-opt",
        "no-new-privileges",
        "--cap-drop=all",
        "--pids-limit",
        "4096",
        "--name",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(run.name.into());
    if !run.network {
        args.push("--network".into());
        args.push("none".into());
    }
    let mut mount = |path: &Path| {
        let mut spec = OsString::from(path);
        spec.push(":");
        spec.push(path);
        args.push("--volume".into());
        args.push(spec);
    };
    mount(run.workspace);
    if let Some(home) = run.agent_home {
        mount(home);
    }
    args.push("--workdir".into());
    args.push(run.workspace.into());
    args.push("--env-file".into());
    args.push(run.env_file.into());
    if run.gpu {
        args.push("--device".into());
        args.push("nvidia.com/gpu=0".into());
    }
    args.push(tag.into());
    // The run ends itself when its time is up; the caller removes the container if even that
    // does not happen.
    for a in ["timeout", "--kill-after=5"] {
        args.push(a.into());
    }
    args.push(run.timeout_seconds.to_string().into());
    args.push(run.interpreter.into());
    args.push(run.script.into());
    args
}

/// The run's command, or None where Podman cannot be reached.
pub fn command(run: &Run<'_>) -> Option<tokio::process::Command> {
    let mut c = podman()?;
    c.args(run_args(run, &image_tag()));
    Some(c)
}

/// Removes a run's container (when it outlived its time).
pub async fn remove(name: &str) {
    if let Some(mut c) = podman() {
        c.args(["rm", "--force", "--ignore", name]);
        let _ = quiet(c).await;
    }
}

/// Writes the run's environment as Podman reads it: one `KEY=value` a line. Values that span
/// lines cannot be written so and are left out.
pub fn write_env_file(path: &Path, vars: &[(String, String)]) -> Result<(), String> {
    let text: String = vars
        .iter()
        .filter(|(k, v)| {
            !k.is_empty() && !k.contains('=') && !v.contains('\n') && !v.contains('\r')
        })
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect();
    // It can hold vault secrets for the moment the run starts: readable by the user only.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| e.to_string())
}

/// What the `run_code` description says about the workbench.
pub fn description(gpu: bool) -> String {
    format!(
        "It runs in the agent workbench, a container that sees only this workspace (and your own \
         home folder, which keeps packages you install with pip or npm); the user's files are out of \
         reach, and the network is on only where the user allowed it for you. It has Python 3 with \
         numpy, pandas, scipy, matplotlib, pillow, requests, pyyaml and openpyxl; Node.js; git, \
         curl, jq; ffmpeg and ImageMagick; and Blender for 3D: write a bpy script and run it with \
         bash: `blender -b -P script.py` (headless; render with Cycles{}). Raise timeout_seconds \
         for renders.",
        if gpu {
            " on the GPU, device type OPTIX or CUDA"
        } else {
            " on the CPU; keep samples low"
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_sees_only_its_folders() {
        let run = Run {
            workspace: Path::new("/home/u/.config/cortex/chat/sandbox/c1"),
            agent_home: Some(Path::new("/home/u/.config/cortex/chat/agent-envs/a1")),
            network: false,
            gpu: true,
            interpreter: "python3",
            script: Path::new("/home/u/.config/cortex/chat/sandbox/c1/.cell.py"),
            timeout_seconds: 300,
            env_file: Path::new("/home/u/.config/cortex/chat/sandbox/c1/.cell.env"),
            name: "cortex-run-x",
        };
        let args: Vec<String> = run_args(&run, "localhost/cortex-workbench:abc")
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let joined = args.join(" ");
        assert!(joined.contains("--network none"));
        assert!(joined.contains("--userns=keep-id"));
        assert!(joined.contains("--init"));
        assert!(joined.contains("--cap-drop=all"));
        assert!(joined.contains(
            "--volume /home/u/.config/cortex/chat/sandbox/c1:/home/u/.config/cortex/chat/sandbox/c1"
        ));
        assert!(joined.contains("--volume /home/u/.config/cortex/chat/agent-envs/a1:/home/u/.config/cortex/chat/agent-envs/a1"));
        assert_eq!(
            args.iter().filter(|a| *a == "--volume").count(),
            2,
            "no other folder is mounted"
        );
        assert!(joined.contains("--device nvidia.com/gpu=0"));
        assert!(joined.ends_with("localhost/cortex-workbench:abc timeout --kill-after=5 300 python3 /home/u/.config/cortex/chat/sandbox/c1/.cell.py"));
        // With the network allowed, Podman's own default network is used.
        let open = Run {
            network: true,
            gpu: false,
            ..run
        };
        let joined: String = run_args(&open, "t")
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!joined.contains("--network") && !joined.contains("nvidia"));
    }

    #[test]
    fn env_files_skip_what_they_cannot_hold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env");
        write_env_file(
            &path,
            &[
                ("HOME".into(), "/w".into()),
                ("MULTI".into(), "a\nb".into()),
                ("TOKEN".into(), "x=y".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "HOME=/w\nTOKEN=x=y\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the user can read it");
        }
    }

    #[test]
    fn the_tag_follows_the_recipe() {
        let tag = image_tag();
        assert!(
            tag.starts_with("localhost/cortex-workbench:")
                && tag.len() == "localhost/cortex-workbench:".len() + 12
        );
        assert!(CONTAINERFILE.contains("blender"));
    }
}
