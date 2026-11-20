//! A coding session's folder: its files, the uncommitted changes, commits and new worktrees.

use super::*;
use crate::chat::error::ChatError;

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
/// Files listed at most, and bytes of one file shown.
pub(super) const MAX_FILES: usize = 5_000;
pub(super) const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Folders that are noise to browse.
pub(super) const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".cache",
];

/// The folder's files, relative and sorted; git's own list when it is a repository, so ignored
/// files stay out.
pub fn files(dir: &str) -> (Vec<String>, bool) {
    let git = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(dir)
        .output();
    let mut out: Vec<String> = match git {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        _ => walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_entry(|e| {
                !(e.file_type().is_dir()
                    && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
            })
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                e.path()
                    .strip_prefix(dir)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .take(MAX_FILES + 1)
            .collect(),
    };
    out.sort();
    let truncated = out.len() > MAX_FILES;
    out.truncate(MAX_FILES);
    (out, truncated)
}

/// One file of the folder as text; refuses paths that leave it.
pub fn read_file(dir: &str, path: &str) -> Result<String, ChatError> {
    let root = std::fs::canonicalize(dir).map_err(ChatError::internal)?;
    let full = std::fs::canonicalize(root.join(path))
        .map_err(|_| "No such file.".to_string())
        .map_err(ChatError::Invalid)?;
    if !full.starts_with(&root) {
        return Err(ChatError::Invalid(
            "That file is outside the session's folder.".into(),
        ));
    }
    let meta = std::fs::metadata(&full).map_err(ChatError::internal)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(ChatError::Invalid(format!(
            "The file is {} KB; only files up to {} KB are shown here.",
            meta.len() / 1024,
            MAX_FILE_BYTES / 1024
        )));
    }
    let bytes = crate::chat::at_rest::read(&full).map_err(ChatError::internal)?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(ChatError::Invalid("Binary file.".into()));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn changes(dir: &str) -> Changes {
    let path = Path::new(dir);
    let Some(branch) = git(path, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return Changes {
            git: false,
            branch: None,
            files: Vec::new(),
            diff: String::new(),
            truncated: false,
        };
    };
    // A repository without commits has no HEAD to compare with; diff the index instead.
    let base: &[&str] = if git(path, &["rev-parse", "--verify", "-q", "HEAD"]).is_some() {
        &["HEAD"]
    } else {
        &[]
    };
    let with_base =
        |args: &[&str]| -> Vec<String> { args.iter().chain(base).map(|s| s.to_string()).collect() };
    let run = |args: Vec<String>| {
        git(path, &args.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default()
    };

    let numstat = run(with_base(&["diff", "--numstat", "-M"]));
    let names = run(with_base(&["diff", "--name-status", "-M"]));
    let mut files = parse(&numstat, &names);
    for untracked in run(vec![
        "ls-files".into(),
        "--others".into(),
        "--exclude-standard".into(),
    ])
    .lines()
    .filter(|l| !l.is_empty())
    {
        let additions = crate::chat::at_rest::read_to_string(path.join(untracked))
            .map(|t| t.lines().count() as u32)
            .unwrap_or(0);
        files.push(ChangedFile {
            path: untracked.to_string(),
            status: "untracked".into(),
            additions,
            deletions: 0,
        });
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
    Changes {
        git: true,
        branch: Some(branch.trim().to_string()),
        files,
        diff,
        truncated,
    }
}

/// Commits every change in `dir` (untracked files included) with `message`; returns the commit
/// summary line.
pub fn commit(dir: &str, message: &str) -> Result<String, ChatError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(ChatError::Invalid("Write a commit message.".into()));
    }
    let path = Path::new(dir);
    if git(path, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Err(ChatError::Invalid(
            "This folder is not a git repository.".into(),
        ));
    }
    let run = |args: &[&str]| -> Result<String, ChatError> {
        let out = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .map_err(ChatError::internal)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(ChatError::Invalid(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    };
    run(&["add", "-A"])?;
    run(&["commit", "-q", "-m", message])?;
    Ok(run(&["log", "-1", "--oneline", "--no-decorate"])?
        .trim()
        .to_string())
}

/// A session's own copy of a repository (Vibe's worktrees): a git worktree beside it, on a new
/// branch from the current commit, so the session's changes stay apart from the main checkout.
/// Returns the worktree's folder and branch.
pub fn create_worktree(dir: &str, name: &str) -> Result<(String, String), ChatError> {
    let path = Path::new(dir);
    let top = git(path, &["rev-parse", "--show-toplevel"])
        .map(|t| t.trim().to_string())
        .ok_or_else(|| ChatError::Invalid("This folder is not a git repository.".into()))?;
    let top = Path::new(&top);
    let repo = top
        .file_name()
        .map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
    let mut slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|w| !w.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = chrono::Local::now().format("%Y%m%d-%H%M").to_string();
    }
    let parent = top.parent().ok_or_else(|| {
        ChatError::Invalid("The repository has no parent folder for a worktree.".into())
    })?;
    let (mut folder, mut branch) = (
        parent.join(format!("{repo}-{slug}")),
        format!("cortex/{slug}"),
    );
    let mut n = 2;
    while folder.exists()
        || git(
            top,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .is_some_and(|o| !o.trim().is_empty())
    {
        folder = parent.join(format!("{repo}-{slug}-{n}"));
        branch = format!("cortex/{slug}-{n}");
        n += 1;
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(top)
        .args(["worktree", "add", "-b", &branch])
        .arg(&folder)
        .env("LC_ALL", "C")
        .output()
        .map_err(ChatError::internal)?;
    if !out.status.success() {
        return Err(ChatError::Invalid(format!(
            "The worktree could not be made: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok((folder.to_string_lossy().into_owned(), branch))
}
