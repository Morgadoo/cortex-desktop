//! Coding sessions: what the model is told about the repository it works in, and the working
//! tree's changes for the Changes panel.
//!
//! Like Mistral Vibe and other coding agents, a session reads the project's own agent
//! instructions (`AGENTS.md`, `CLAUDE.md`, ...) and knows its git branch and status, so the
//! model starts oriented instead of spending steps to find out.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// Files a project keeps its instructions for coding agents in, read in this order.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", ".github/copilot-instructions.md", "VIBE.md"];
/// Characters of project instructions put in the prompt.
const INSTRUCTION_CHARS: usize = 10_000;
/// Status lines put in the prompt.
const STATUS_LINES: usize = 30;
/// Bytes of diff sent to the Changes panel.
const DIFF_BYTES: usize = 400_000;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args).env("GIT_OPTIONAL_LOCKS", "0").env("LC_ALL", "C");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = command.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The system-prompt section for a coding session in `dir`.
pub fn prompt(dir: &str) -> String {
    let path = Path::new(dir);
    let mut out = format!(
        "\n\nThis is a coding session in the folder `{dir}`. Read the code before changing it, \
         keep changes focused, run the project's own build and test commands to check your work, \
         and report what you changed."
    );
    if let Some(branch) = git(path, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        out.push_str(&format!("\n\nGit: on branch `{}`.", branch.trim()));
        let status = git(path, &["status", "--porcelain=v1"]).unwrap_or_default();
        let lines: Vec<&str> = status.lines().collect();
        if lines.is_empty() {
            out.push_str(" The working tree is clean.");
        } else {
            out.push_str(&format!(" Uncommitted changes ({} files):\n", lines.len()));
            for line in lines.iter().take(STATUS_LINES) {
                out.push_str(&format!("{line}\n"));
            }
            if lines.len() > STATUS_LINES {
                out.push_str(&format!("… and {} more\n", lines.len() - STATUS_LINES));
            }
        }
        if let Some(log) = git(path, &["log", "--oneline", "-5", "--no-decorate"]).filter(|l| !l.trim().is_empty()) {
            out.push_str(&format!("\nRecent commits:\n{}", log.trim_end()));
        }
        out.push_str("\nDo not commit, push or rewrite history unless the user asks.");
    }
    for name in INSTRUCTION_FILES {
        let Ok(text) = std::fs::read_to_string(path.join(name)) else { continue };
        let text: String = text.chars().take(INSTRUCTION_CHARS).collect();
        out.push_str(&format!(
            "\n\nThe project's instructions for coding agents, from `{name}` (follow them):\n<project_instructions>\n{}\n</project_instructions>",
            text.trim()
        ));
        break;
    }
    out
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    /// `modified`, `added`, `deleted`, `renamed` or `untracked`.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub git: bool,
    pub branch: Option<String>,
    pub files: Vec<ChangedFile>,
    /// The unified diff against HEAD, cut to a size the page can show.
    pub diff: String,
    pub truncated: bool,
}

/// The working tree's changes against HEAD, untracked files included.
pub fn changes(dir: &str) -> Changes {
    let path = Path::new(dir);
    let Some(branch) = git(path, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return Changes { git: false, branch: None, files: Vec::new(), diff: String::new(), truncated: false };
    };
    // A repository without commits has no HEAD to compare with; diff the index instead.
    let base: &[&str] = if git(path, &["rev-parse", "--verify", "-q", "HEAD"]).is_some() { &["HEAD"] } else { &[] };
    let with_base = |args: &[&str]| -> Vec<String> { args.iter().chain(base).map(|s| s.to_string()).collect() };
    let run = |args: Vec<String>| git(path, &args.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default();

    let numstat = run(with_base(&["diff", "--numstat", "-M"]));
    let names = run(with_base(&["diff", "--name-status", "-M"]));
    let mut files = parse(&numstat, &names);
    for untracked in run(vec!["ls-files".into(), "--others".into(), "--exclude-standard".into()]).lines().filter(|l| !l.is_empty()) {
        let additions = std::fs::read_to_string(path.join(untracked)).map(|t| t.lines().count() as u32).unwrap_or(0);
        files.push(ChangedFile { path: untracked.to_string(), status: "untracked".into(), additions, deletions: 0 });
    }
    let mut diff = run(with_base(&["diff", "-M", "--no-color", "--no-ext-diff"]));
    let truncated = diff.len() > DIFF_BYTES;
    if truncated {
        let mut cut = DIFF_BYTES;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        diff.truncate(cut);
    }
    Changes { git: true, branch: Some(branch.trim().to_string()), files, diff, truncated }
}

fn parse(numstat: &str, names: &str) -> Vec<ChangedFile> {
    let statuses: Vec<(String, String)> = names
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let code = parts.next()?;
            let path = parts.last()?;
            let status = match code.chars().next()? {
                'A' => "added",
                'D' => "deleted",
                'R' => "renamed",
                _ => "modified",
            };
            Some((path.to_string(), status.to_string()))
        })
        .collect();
    numstat
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let additions = parts.next()?.parse().unwrap_or(0);
            let deletions = parts.next()?.parse().unwrap_or(0);
            let raw = parts.next()?;
            // A rename reads `old => new` or `dir/{old => new}/file`.
            let path = if let (Some(open), Some(close)) = (raw.find('{'), raw.find('}')) {
                let inner = &raw[open + 1..close];
                let new = inner.split(" => ").last().unwrap_or(inner);
                format!("{}{}{}", &raw[..open], new, &raw[close + 1..]).replace("//", "/")
            } else {
                raw.split(" => ").last().unwrap_or(raw).to_string()
            };
            let status = statuses.iter().find(|(p, _)| *p == path).map_or("modified".to_string(), |(_, s)| s.clone());
            Some(ChangedFile { path, status, additions, deletions })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().unwrap();
        assert!(ok.success());
    }

    #[test]
    fn reads_repository_state_and_changes() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("AGENTS.md"), "Run `make test` before finishing.").unwrap();
        sh(dir, &["add", "."]);
        sh(dir, &["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x\ny\n").unwrap();

        let text = prompt(dir.to_str().unwrap());
        assert!(text.contains("on branch `main`"));
        assert!(text.contains(" M a.txt"));
        assert!(text.contains("first"));
        assert!(text.contains("Run `make test`"));

        let c = changes(dir.to_str().unwrap());
        assert!(c.git);
        assert_eq!(c.branch.as_deref(), Some("main"));
        assert_eq!(c.files, vec![
            ChangedFile { path: "a.txt".into(), status: "modified".into(), additions: 2, deletions: 1 },
            ChangedFile { path: "new.txt".into(), status: "untracked".into(), additions: 2, deletions: 0 },
        ]);
        assert!(c.diff.contains("+TWO"));

        let plain = tempfile::tempdir().unwrap();
        assert!(!changes(plain.path().to_str().unwrap()).git);
        assert!(!prompt(plain.path().to_str().unwrap()).contains("Git:"));
    }

    #[test]
    fn parses_renames() {
        let files = parse("0\t0\tsrc/{old.rs => new.rs}\n", "R100\tsrc/old.rs\tsrc/new.rs\n");
        assert_eq!(files[0].path, "src/new.rs");
        assert_eq!(files[0].status, "renamed");
    }
}
