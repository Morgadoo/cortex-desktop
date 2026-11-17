//! Code execution: the model runs Python, JavaScript or shell code in a workspace of its own.
//!
//! Each conversation has a workspace folder, so files one run writes (a CSV, a chart) are there
//! for the next and for the user. Runs are isolated with bubblewrap where it is available: the
//! home folder is hidden, the network is off and only the workspace is writable. Without it, the
//! run still gets a cleared environment (no API keys) and a time limit, and the user approves
//! each run unless they allowed them all.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use serde_json::{json, Value};

use super::store::{chat_dir, valid_id};
use super::tools::{ChatTool, Outcome};

pub const RUN_CODE: &str = "run_code";

/// Seconds a run may take unless the model asks for more, and the most it may ask for.
const DEFAULT_TIMEOUT: u64 = 30;
const MAX_TIMEOUT: u64 = 120;
/// Characters kept of each output stream; the tail is what matters when a run fails.
const OUTPUT_CHARS: usize = 12_000;
/// Files listed back to the model after a run.
const MAX_FILES: usize = 40;

/// Set when the user chose "Always allow" for code runs; lasts until the app quits.
static ALWAYS: AtomicBool = AtomicBool::new(false);

pub fn always_allowed() -> bool {
    ALWAYS.load(Ordering::Relaxed)
}

pub fn allow_always() {
    ALWAYS.store(true, Ordering::Relaxed);
}

struct Language {
    name: &'static str,
    programs: &'static [&'static str],
    ext: &'static str,
}

const LANGUAGES: &[Language] = &[
    Language { name: "python", programs: &["python3", "python"], ext: "py" },
    Language { name: "javascript", programs: &["node", "deno", "bun"], ext: "js" },
    Language { name: "bash", programs: &["bash", "sh"], ext: "sh" },
];

fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        let exe = dir.join(format!("{name}.exe"));
        exe.is_file().then_some(exe)
    })
}

/// The interpreters on this machine, by language.
fn interpreters() -> &'static HashMap<&'static str, PathBuf> {
    static FOUND: OnceLock<HashMap<&'static str, PathBuf>> = OnceLock::new();
    FOUND.get_or_init(|| {
        LANGUAGES
            .iter()
            .filter_map(|l| l.programs.iter().find_map(|p| find_program(p)).map(|path| (l.name, path)))
            .collect()
    })
}

/// Whether bubblewrap runs here (it needs user namespaces, which some systems turn off).
fn bubblewrap() -> Option<&'static PathBuf> {
    static BWRAP: OnceLock<Option<PathBuf>> = OnceLock::new();
    BWRAP
        .get_or_init(|| {
            if !cfg!(target_os = "linux") {
                return None;
            }
            let bwrap = find_program("bwrap")?;
            let works = std::process::Command::new(&bwrap)
                .args(["--ro-bind", "/", "/", "--unshare-all", "--die-with-parent", "--", "true"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            works.then_some(bwrap)
        })
        .as_ref()
}

/// The languages a run may use, for the tool description; empty when none is installed.
pub fn languages() -> Vec<&'static str> {
    LANGUAGES.iter().map(|l| l.name).filter(|n| interpreters().contains_key(n)).collect()
}

/// The `run_code` tool, when an interpreter is installed.
pub fn definition() -> Option<ChatTool> {
    let langs = languages();
    if langs.is_empty() {
        return None;
    }
    let isolation = if bubblewrap().is_some() {
        "It runs isolated: no network, the user's files are hidden, and only the workspace is writable."
    } else {
        "It runs on the user's machine with a cleared environment."
    };
    Some(ChatTool {
        name: RUN_CODE.into(),
        description: format!(
            "Run code and get its output: for calculations, data analysis, parsing files, \
             converting formats and making charts. Runs in this conversation's workspace folder, \
             which keeps files between runs; save charts or results there as files (for Python \
             charts, matplotlib with plt.savefig('chart.png')) and they are shown to the user. \
             Print what you need to see. {isolation} Languages: {}.",
            langs.join(", ")
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "language": { "type": "string", "enum": langs },
                "code": { "type": "string", "description": "The complete program." },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT, "default": DEFAULT_TIMEOUT }
            },
            "required": ["language", "code"]
        }),
        connector_id: None,
        tool_name: RUN_CODE.into(),
        read_only: false,
    })
}

/// A conversation's workspace folder.
pub fn workspace(config_dir: &Path, conversation_id: &str) -> Result<PathBuf, String> {
    if !valid_id(conversation_id) {
        return Err("invalid conversation id".into());
    }
    Ok(chat_dir(config_dir).join("sandbox").join(conversation_id))
}

#[derive(Debug, Serialize)]
struct FileInfo {
    name: String,
    bytes: u64,
    /// Written or changed by this run.
    new: bool,
}

fn snapshot(dir: &Path) -> HashMap<String, (u64, Option<SystemTime>)> {
    walkdir::WalkDir::new(dir)
        .max_depth(3)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(dir).ok()?.to_string_lossy().replace('\\', "/");
            if rel.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some((rel, (meta.len(), meta.modified().ok())))
        })
        .collect()
}

fn tail(text: &[u8]) -> String {
    let text = String::from_utf8_lossy(text);
    let count = text.chars().count();
    if count <= OUTPUT_CHARS {
        return text.into_owned();
    }
    let kept: String = text.chars().skip(count - OUTPUT_CHARS).collect();
    format!("[… {} earlier characters cut]\n{kept}", count - OUTPUT_CHARS)
}

/// Runs `code`, returning a JSON report of its exit, output and files.
pub async fn run(config_dir: &Path, conversation_id: &str, args: &Value) -> Outcome {
    let err = |text: String| Outcome { text, is_error: true };
    let language = args.get("language").and_then(Value::as_str).unwrap_or("python").to_ascii_lowercase();
    let language = match language.as_str() {
        "py" | "python3" => "python".to_string(),
        "js" | "node" | "typescript" => "javascript".to_string(),
        "sh" | "shell" => "bash".to_string(),
        _ => language,
    };
    let Some(code) = args.get("code").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) else {
        return err("Give the code to run.".into());
    };
    let Some(lang) = LANGUAGES.iter().find(|l| l.name == language) else {
        return err(format!("Unknown language `{language}`. Use one of: {}.", languages().join(", ")));
    };
    let Some(program) = interpreters().get(lang.name) else {
        return err(format!("{} is not installed on this computer.", lang.name));
    };
    let timeout = args.get("timeout_seconds").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT);
    let dir = match workspace(config_dir, conversation_id) {
        Ok(d) => d,
        Err(e) => return err(e),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return err(format!("Could not create the workspace: {e}"));
    }
    let script = dir.join(format!(".cell.{}", lang.ext));
    if let Err(e) = std::fs::write(&script, code) {
        return err(format!("Could not write the program: {e}"));
    }
    let before = snapshot(&dir);

    let isolated = bubblewrap().is_some();
    let mut command = match bubblewrap() {
        Some(bwrap) => {
            let mut c = tokio::process::Command::new(bwrap);
            c.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);
            // Hide the user's files, then put the workspace back where it was.
            if let Some(home) = dirs_home() {
                c.arg("--tmpfs").arg(&home);
                // An interpreter installed under the home folder (pyenv, nvm) stays reachable.
                if let Some(prefix) = program.parent().and_then(Path::parent).filter(|p| p.starts_with(&home)) {
                    c.arg("--ro-bind").arg(prefix).arg(prefix);
                }
            }
            c.arg("--bind").arg(&dir).arg(&dir).arg("--chdir").arg(&dir);
            c.args(["--unshare-all", "--die-with-parent", "--new-session", "--"]);
            c.arg(program).arg(&script);
            c
        }
        None => {
            let mut c = tokio::process::Command::new(program);
            c.arg(&script).current_dir(&dir);
            c
        }
    };
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &dir)
        .env("TMPDIR", if isolated { PathBuf::from("/tmp") } else { std::env::temp_dir() })
        .env("LANG", "C.UTF-8")
        .env("MPLBACKEND", "Agg")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    for key in ["SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT"] {
        if let Some(v) = std::env::var_os(key) {
            command.env(key, v);
        }
    }

    let started = Instant::now();
    let child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return err(format!("Could not start {}: {e}", lang.name)),
    };
    let (exit_code, stdout, stderr, timed_out) =
        match tokio::time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
            Ok(Ok(out)) => (out.status.code(), tail(&out.stdout), tail(&out.stderr), false),
            Ok(Err(e)) => return err(format!("The run failed: {e}")),
            // Dropping the future kills the process.
            Err(_) => (None, String::new(), String::new(), true),
        };
    let _ = std::fs::remove_file(&script);

    let after = snapshot(&dir);
    let mut files: Vec<FileInfo> = after
        .iter()
        .map(|(name, (bytes, modified))| FileInfo {
            name: name.clone(),
            bytes: *bytes,
            new: before.get(name).is_none_or(|(b, m)| b != bytes || m != modified),
        })
        .collect();
    files.sort_by(|a, b| b.new.cmp(&a.new).then(a.name.cmp(&b.name)));
    files.truncate(MAX_FILES);

    let report = json!({
        "language": lang.name,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "seconds": (started.elapsed().as_millis() as f64 / 1000.0),
        "stdout": stdout,
        "stderr": stderr,
        "files": files,
        "isolated": isolated,
    });
    let failed = timed_out || exit_code != Some(0);
    let mut text = serde_json::to_string_pretty(&report).unwrap_or_default();
    if timed_out {
        text.push_str(&format!("\n\nThe run was stopped after {timeout} seconds."));
    }
    Outcome { text, is_error: failed }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute() && p != Path::new("/"))
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("csv") => "text/csv; charset=utf-8",
        Some("json") => "application/json",
        Some("txt" | "md" | "log") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serves `lumoapp://localhost/sandbox/<conversation>/<file>` so the chat can show a run's
/// charts. Only files inside that conversation's workspace are served.
pub fn serve(config_dir: &Path, rest: &str) -> tauri::http::Response<Vec<u8>> {
    let not_found = || {
        tauri::http::Response::builder().status(404).body(b"Not found".to_vec()).unwrap_or_default()
    };
    let rest = rest.split('?').next().unwrap_or_default();
    let Some((conversation, file)) = rest.split_once('/') else { return not_found() };
    let file = percent_decode(file);
    if file.is_empty() || file.split('/').any(|seg| seg.is_empty() || seg == ".." || seg.starts_with('.')) || file.contains('\\') {
        return not_found();
    }
    let Ok(dir) = workspace(config_dir, conversation) else { return not_found() };
    let path = dir.join(&file);
    let (Ok(root), Ok(real)) = (dir.canonicalize(), path.canonicalize()) else { return not_found() };
    if !real.starts_with(&root) {
        return not_found();
    }
    match std::fs::read(&real) {
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", content_type(&file))
            // An SVG could carry script; it is shown as an image, never as a page.
            .header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'")
            .header("X-Content-Type-Options", "nosniff")
            .header("Cache-Control", "no-store")
            .body(bytes)
            .unwrap_or_default(),
        Err(_) => not_found(),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_code_in_the_workspace_and_lists_its_files() {
        if !languages().contains(&"python") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = run(dir.path(), "conv", &json!({
            "language": "python",
            "code": "import os\nopen('out.csv','w').write('a,b\\n1,2\\n')\nprint(sum(range(10)))\nprint('KEY' in ''.join(os.environ))"
        }))
        .await;
        assert!(!out.is_error, "{}", out.text);
        let report: Value = serde_json::from_str(&out.text).unwrap();
        assert_eq!(report["stdout"], "45\nFalse\n");
        assert_eq!(report["files"][0]["name"], "out.csv");
        assert_eq!(report["files"][0]["new"], true);
        // The program file itself is not left behind.
        assert!(!workspace(dir.path(), "conv").unwrap().join(".cell.py").exists());

        let served = serve(dir.path(), "conv/out.csv");
        assert_eq!(served.status(), 200);
        assert_eq!(served.body(), b"a,b\n1,2\n");
        assert_eq!(serve(dir.path(), "conv/../conv/out.csv").status(), 404);
        assert_eq!(serve(dir.path(), "../x/out.csv").status(), 404);

        let failed = run(dir.path(), "conv", &json!({ "language": "python", "code": "raise SystemExit(3)" })).await;
        assert!(failed.is_error);
        let slow = run(dir.path(), "conv", &json!({ "language": "python", "code": "import time\ntime.sleep(5)", "timeout_seconds": 1 })).await;
        assert!(slow.is_error && slow.text.contains("\"timed_out\": true"));
    }

    #[test]
    fn decodes_file_names() {
        assert_eq!(percent_decode("my%20chart.png"), "my chart.png");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
