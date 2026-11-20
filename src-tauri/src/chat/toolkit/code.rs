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
const INSTRUCTION_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".github/copilot-instructions.md",
    "VIBE.md",
];
/// Characters of project instructions put in the prompt.
const INSTRUCTION_CHARS: usize = 10_000;
/// Status lines put in the prompt.
const STATUS_LINES: usize = 30;
/// Bytes of diff sent to the Changes panel.
const DIFF_BYTES: usize = 400_000;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = command.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
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
        if let Some(log) =
            git(path, &["log", "--oneline", "-5", "--no-decorate"]).filter(|l| !l.trim().is_empty())
        {
            out.push_str(&format!("\nRecent commits:\n{}", log.trim_end()));
        }
        out.push_str("\nDo not commit, push or rewrite history unless the user asks.");
    }
    for name in INSTRUCTION_FILES {
        let Ok(text) = crate::chat::at_rest::read_to_string(path.join(name)) else {
            continue;
        };
        let text: String = text.chars().take(INSTRUCTION_CHARS).collect();
        out.push_str(&format!(
            "\n\nThe project's instructions for coding agents, from `{name}` (follow them):\n<project_instructions>\n{}\n</project_instructions>",
            text.trim()
        ));
        break;
    }
    out
}

fn parse(numstat: &str, names: &str) -> Vec<ChangedFile> {
    let statuses: Vec<(String, String)> = names
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let code = parts.next()?;
            let path = parts.next_back()?;
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
            let status = statuses
                .iter()
                .find(|(p, _)| *p == path)
                .map_or("modified".to_string(), |(_, s)| s.clone());
            Some(ChangedFile {
                path,
                status,
                additions,
                deletions,
            })
        })
        .collect()
}

mod pull_requests;
mod workspace;
pub use pull_requests::*;
pub use workspace::*;

#[cfg(test)]
mod tests;
