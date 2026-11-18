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
        let Ok(text) = crate::chat::at_rest::read_to_string(path.join(name)) else { continue };
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
/// Files listed at most, and bytes of one file shown.
const MAX_FILES: usize = 5_000;
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Folders that are noise to browse.
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".venv", "venv", "__pycache__", ".next", ".cache"];

/// The folder's files, relative and sorted; git's own list when it is a repository, so ignored
/// files stay out.
pub fn files(dir: &str) -> (Vec<String>, bool) {
    let git = std::process::Command::new("git").args(["ls-files", "--cached", "--others", "--exclude-standard"]).current_dir(dir).output();
    let mut out: Vec<String> = match git {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect(),
        _ => walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref())))
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.path().strip_prefix(dir).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
            .take(MAX_FILES + 1)
            .collect(),
    };
    out.sort();
    let truncated = out.len() > MAX_FILES;
    out.truncate(MAX_FILES);
    (out, truncated)
}

/// One file of the folder as text; refuses paths that leave it.
pub fn read_file(dir: &str, path: &str) -> Result<String, String> {
    let root = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let full = std::fs::canonicalize(root.join(path)).map_err(|_| "No such file.".to_string())?;
    if !full.starts_with(&root) {
        return Err("That file is outside the session's folder.".into());
    }
    let meta = std::fs::metadata(&full).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("The file is {} KB; only files up to {} KB are shown here.", meta.len() / 1024, MAX_FILE_BYTES / 1024));
    }
    let bytes = crate::chat::at_rest::read(&full).map_err(|e| e.to_string())?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err("Binary file.".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

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
        let additions = crate::chat::at_rest::read_to_string(path.join(untracked)).map(|t| t.lines().count() as u32).unwrap_or(0);
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

/// Commits every change in `dir` (untracked files included) with `message`; returns the commit
/// summary line.
pub fn commit(dir: &str, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message.".into());
    }
    let path = Path::new(dir);
    if git(path, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Err("This folder is not a git repository.".into());
    }
    let run = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git").arg("-C").arg(path).args(args).env("LC_ALL", "C").output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    run(&["add", "-A"])?;
    run(&["commit", "-q", "-m", message])?;
    Ok(run(&["log", "-1", "--oneline", "--no-decorate"])?.trim().to_string())
}

/// A session's own copy of a repository (Vibe's worktrees): a git worktree beside it, on a new
/// branch from the current commit, so the session's changes stay apart from the main checkout.
/// Returns the worktree's folder and branch.
pub fn create_worktree(dir: &str, name: &str) -> Result<(String, String), String> {
    let path = Path::new(dir);
    let top = git(path, &["rev-parse", "--show-toplevel"]).map(|t| t.trim().to_string()).ok_or("This folder is not a git repository.")?;
    let top = Path::new(&top);
    let repo = top.file_name().map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
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
    let parent = top.parent().ok_or("The repository has no parent folder for a worktree.")?;
    let (mut folder, mut branch) = (parent.join(format!("{repo}-{slug}")), format!("lumo/{slug}"));
    let mut n = 2;
    while folder.exists() || git(top, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_some_and(|o| !o.trim().is_empty()) {
        folder = parent.join(format!("{repo}-{slug}-{n}"));
        branch = format!("lumo/{slug}-{n}");
        n += 1;
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(top)
        .args(["worktree", "add", "-b", &branch])
        .arg(&folder)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("The worktree could not be made: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok((folder.to_string_lossy().into_owned(), branch))
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub branch: String,
    /// Where to see or finish the pull request: the PR itself (via `gh`) or GitHub's compare page.
    pub url: Option<String>,
    /// `gh` when the GitHub CLI opened it, `browser` when the compare page finishes it.
    pub via: String,
}

/// The pull request for a branch, as its host reports it (LibreChat's PR status for a workspace).
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `open`, `draft`, `merged` or `closed`.
    pub state: String,
    /// `approved`, `changes requested`, `review required`, or empty.
    pub review: String,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    /// `passed`, `failed`, `running`, `skipped` or `pending`.
    pub state: String,
    pub url: String,
}

/// A check's outcome from `gh`'s `statusCheckRollup` entry (a check run or a commit status).
fn check_state(v: &serde_json::Value) -> String {
    let s = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_ascii_uppercase();
    match (s("status").as_str(), s("conclusion").as_str(), s("state").as_str()) {
        (_, "SUCCESS", _) | (_, _, "SUCCESS") => "passed",
        (_, "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE", _) | (_, _, "FAILURE" | "ERROR") => "failed",
        (_, "SKIPPED" | "NEUTRAL", _) => "skipped",
        ("IN_PROGRESS" | "QUEUED" | "WAITING" | "PENDING" | "REQUESTED", _, _) => "running",
        (_, _, "PENDING" | "EXPECTED") => "pending",
        _ => "pending",
    }
    .into()
}

/// Reads `gh pr view --json …` output.
pub fn parse_gh_pr(json: &str) -> Option<PrStatus> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let state = match (v["state"].as_str().unwrap_or_default(), v["isDraft"].as_bool().unwrap_or(false)) {
        ("OPEN", true) => "draft",
        ("OPEN", false) => "open",
        ("MERGED", _) => "merged",
        _ => "closed",
    };
    let review = match v["reviewDecision"].as_str().unwrap_or_default() {
        "APPROVED" => "approved",
        "CHANGES_REQUESTED" => "changes requested",
        "REVIEW_REQUIRED" => "review required",
        _ => "",
    };
    let checks = v["statusCheckRollup"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Check {
            name: c["name"].as_str().or(c["context"].as_str()).unwrap_or("check").to_string(),
            state: check_state(c),
            url: c["detailsUrl"].as_str().or(c["targetUrl"].as_str()).unwrap_or_default().to_string(),
        })
        .collect();
    Some(PrStatus { number: v["number"].as_u64()?, title: v["title"].as_str().unwrap_or_default().into(), url: v["url"].as_str().unwrap_or_default().into(), state: state.into(), review: review.into(), checks })
}

/// Reads `glab mr view -F json` output.
pub fn parse_glab_mr(json: &str) -> Option<PrStatus> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let state = match (v["state"].as_str().unwrap_or_default(), v["draft"].as_bool().unwrap_or(false)) {
        ("opened", true) => "draft",
        ("opened", false) => "open",
        ("merged", _) => "merged",
        _ => "closed",
    };
    let pipeline = &v["head_pipeline"];
    let checks = if pipeline.is_object() {
        let st = match pipeline["status"].as_str().unwrap_or_default() {
            "success" => "passed",
            "failed" | "canceled" => "failed",
            "running" => "running",
            "skipped" => "skipped",
            _ => "pending",
        };
        vec![Check { name: "Pipeline".into(), state: st.into(), url: pipeline["web_url"].as_str().unwrap_or_default().into() }]
    } else {
        Vec::new()
    };
    Some(PrStatus { number: v["iid"].as_u64()?, title: v["title"].as_str().unwrap_or_default().into(), url: v["web_url"].as_str().unwrap_or_default().into(), state: state.into(), review: if v["approved"].as_bool() == Some(true) { "approved".into() } else { String::new() }, checks })
}

/// The pull request of the folder's current branch, through `gh` or `glab`; `None` when there is
/// none or neither tool is installed and signed in. `tools` names the programs (tests swap them).
pub fn pr_status_with(dir: &str, gh: &str, glab: &str) -> Option<PrStatus> {
    let out = Command::new(gh)
        .current_dir(dir)
        .args(["pr", "view", "--json", "number,title,url,state,isDraft,reviewDecision,statusCheckRollup"])
        .output()
        .ok()
        .filter(|o| o.status.success());
    if let Some(pr) = out.and_then(|o| parse_gh_pr(&String::from_utf8_lossy(&o.stdout))) {
        return Some(pr);
    }
    let out = Command::new(glab).current_dir(dir).args(["mr", "view", "-F", "json"]).output().ok().filter(|o| o.status.success())?;
    parse_glab_mr(&String::from_utf8_lossy(&out.stdout))
}

pub fn pr_status(dir: &str) -> Option<PrStatus> {
    pr_status_with(dir, "gh", "glab")
}

/// `owner/repo` of a GitHub remote URL (https or ssh), if it is one.
pub fn github_repo(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("http://github.com/"))
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty() && parts.next().is_none()).then(|| format!("{owner}/{repo}"))
}

/// Host and repository path of a remote URL (https, ssh or scp-style), e.g.
/// `("gitlab.com", "group/sub/app")`.
pub fn remote_location(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = if let Some(r) = remote.strip_prefix("https://").or_else(|| remote.strip_prefix("http://")).or_else(|| remote.strip_prefix("ssh://")) {
        r.to_string()
    } else {
        // scp-like `user@host:path`.
        let (host, path) = remote.split_once(':')?;
        if host.contains('/') || path.starts_with("//") {
            return None;
        }
        format!("{host}/{path}")
    };
    let (host, path) = rest.split_once('/')?;
    let host = host.rsplit('@').next()?.split(':').next()?.to_lowercase();
    let path = path.trim_matches('/');
    (!host.is_empty() && path.contains('/')).then(|| (host, path.to_string()))
}

/// The page that opens a pull (or merge) request on a known kind of host.
pub fn compare_page(remote: &str, base: &str, branch: &str) -> Option<String> {
    let (host, path) = remote_location(remote)?;
    let enc = |s: &str| s.replace('%', "%25").replace(' ', "%20").replace('#', "%23").replace('&', "%26").replace('?', "%3F");
    if host == "github.com" {
        return github_repo(remote).map(|repo| format!("https://github.com/{repo}/compare/{}...{}?expand=1", enc(base), enc(branch)));
    }
    if host.contains("gitlab") {
        return Some(format!(
            "https://{host}/{path}/-/merge_requests/new?merge_request%5Bsource_branch%5D={}&merge_request%5Btarget_branch%5D={}",
            enc(branch),
            enc(base)
        ));
    }
    if host == "bitbucket.org" {
        return Some(format!("https://bitbucket.org/{path}/pull-requests/new?source={}&dest={}", enc(branch), enc(base)));
    }
    if host == "codeberg.org" || host.contains("gitea") || host.contains("forgejo") {
        return Some(format!("https://{host}/{path}/compare/{}...{}", enc(base), enc(branch)));
    }
    None
}

/// The link a host prints while pushing a new branch ("To create a merge request…"): GitLab,
/// Gitea, Forgejo and Bitbucket do this, self-hosted ones included.
pub fn link_from_push(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim_start_matches("remote:").trim();
        let start = line.find("https://").or_else(|| line.find("http://"))?;
        let url = line[start..].split_whitespace().next()?;
        ["merge_requests/new", "pull-requests/new", "/compare/", "/pull/new/", "/pulls/new"].iter().any(|m| url.contains(m)).then(|| url.to_string())
    })
}

/// A branch name from a title: `ai/` plus lowercase words joined by hyphens.
fn branch_name(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|w| !w.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    format!("ai/{}", if slug.is_empty() { "changes".into() } else { slug })
}

/// Pushes the current branch (creating one first when on the default branch) and opens a pull
/// request for it: with the GitHub CLI (or GitLab's `glab`) when it is installed and signed in,
/// otherwise by handing back the host's new-pull-request page, where the user finishes it. The
/// page comes from the remote for GitHub, GitLab, Bitbucket, Codeberg, Gitea and Forgejo, or from
/// the link the host printed during the push.
pub fn open_pull_request(dir: &str, title: &str) -> Result<PullRequest, String> {
    let path = Path::new(dir);
    let run = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git").arg("-C").arg(path).args(args).env("LC_ALL", "C").output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    let remote = run(&["remote", "get-url", "origin"]).map_err(|_| "This repository has no `origin` remote to push to.".to_string())?;
    let default = run(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .and_then(|r| r.strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| "main".into());
    let mut branch = run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == default || branch == "HEAD" {
        branch = branch_name(title);
        run(&["switch", "-c", &branch])?;
    }
    let push = Command::new("git").arg("-C").arg(path).args(["push", "-u", "origin", &branch]).env("LC_ALL", "C").output().map_err(|e| e.to_string())?;
    let push_output = format!("{}\n{}", String::from_utf8_lossy(&push.stdout), String::from_utf8_lossy(&push.stderr));
    if !push.status.success() {
        return Err(format!("Pushing {branch} failed: {}", String::from_utf8_lossy(&push.stderr).trim()));
    }

    let repo = github_repo(&remote);
    // The GitHub CLI, when it is installed and signed in, opens the pull request directly.
    if repo.is_some() {
        let mut gh = Command::new("gh");
        gh.current_dir(path).args(["pr", "create", "--head", &branch, "--base", &default, "--title", title.trim(), "--fill"]);
        if let Ok(out) = gh.output() {
            if out.status.success() {
                let url = String::from_utf8_lossy(&out.stdout).lines().rev().find(|l| l.starts_with("https://")).map(str::to_string);
                return Ok(PullRequest { branch, url, via: "gh".into() });
            }
        }
    }
    // GitLab's CLI does the same for GitLab remotes.
    if remote_location(&remote).is_some_and(|(host, _)| host.contains("gitlab")) {
        let mut glab = Command::new("glab");
        glab.current_dir(path).args(["mr", "create", "--source-branch", &branch, "--target-branch", &default, "--title", title.trim(), "--fill", "--yes"]);
        if let Ok(out) = glab.output() {
            if out.status.success() {
                let url = String::from_utf8_lossy(&out.stdout).split_whitespace().rev().find(|w| w.starts_with("https://")).map(str::to_string);
                return Ok(PullRequest { branch, url, via: "cli".into() });
            }
        }
    }
    let url = compare_page(&remote, &default, &branch).or_else(|| link_from_push(&push_output));
    Ok(PullRequest { via: if url.is_some() { "browser" } else { "none" }.into(), url, branch })
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
            let status = statuses.iter().find(|(p, _)| *p == path).map_or("modified".to_string(), |(_, s)| s.clone());
            Some(ChangedFile { path, status, additions, deletions })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_status_and_checks() {
        let gh = r#"{"number":42,"title":"Use two","url":"https://github.com/me/app/pull/42","state":"OPEN","isDraft":false,"reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[
            {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://x/1"},
            {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://x/2"},
            {"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":""},
            {"__typename":"StatusContext","context":"ci/legacy","state":"PENDING","targetUrl":"https://x/3"}]}"#;
        let pr = parse_gh_pr(gh).unwrap();
        assert_eq!((pr.number, pr.state.as_str(), pr.review.as_str()), (42, "open", "changes requested"));
        let states: Vec<(&str, &str)> = pr.checks.iter().map(|c| (c.name.as_str(), c.state.as_str())).collect();
        assert_eq!(states, [("test", "passed"), ("lint", "failed"), ("build", "running"), ("ci/legacy", "pending")]);
        let mr = parse_glab_mr(r#"{"iid":7,"title":"T","web_url":"https://gitlab.com/me/app/-/merge_requests/7","state":"merged","head_pipeline":{"status":"success","web_url":"https://gl/p"}}"#).unwrap();
        assert_eq!((mr.number, mr.state.as_str(), mr.checks[0].state.as_str()), (7, "merged", "passed"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let dir = tempfile::tempdir().unwrap();
            let fake = dir.path().join("gh");
            std::fs::write(&fake, format!("#!/bin/sh\ncat <<'EOF'\n{gh}\nEOF\n")).unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
            let got = pr_status_with(dir.path().to_str().unwrap(), fake.to_str().unwrap(), "/nonexistent/glab").unwrap();
            assert_eq!(got.checks.len(), 4);
            assert!(pr_status_with(dir.path().to_str().unwrap(), "/nonexistent/gh", "/nonexistent/glab").is_none());
        }
    }

    #[test]
    fn lists_and_reads_files_inside_the_folder_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        crate::chat::at_rest::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        crate::chat::at_rest::write(dir.path().join("node_modules/x/i.js"), "x").unwrap();
        let root = dir.path().to_str().unwrap();
        let (list, truncated) = files(root);
        assert!(list.contains(&"src/main.rs".to_string()) && !truncated);
        assert!(!list.iter().any(|f| f.starts_with("node_modules")));
        assert_eq!(read_file(root, "src/main.rs").unwrap(), "fn main() {}");
        assert!(read_file(root, "../../etc/passwd").is_err());
    }

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
        crate::chat::at_rest::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        crate::chat::at_rest::write(dir.join("AGENTS.md"), "Run `make test` before finishing.").unwrap();
        sh(dir, &["add", "."]);
        sh(dir, &["commit", "-q", "-m", "first"]);
        crate::chat::at_rest::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        crate::chat::at_rest::write(dir.join("new.txt"), "x\ny\n").unwrap();

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

        // Committing takes everything, untracked files included.
        sh(dir, &["config", "user.name", "t"]);
        sh(dir, &["config", "user.email", "t@t"]);
        let line = commit(dir.to_str().unwrap(), "Simplify totals").unwrap();
        assert!(line.ends_with("Simplify totals"), "{line}");
        assert!(changes(dir.to_str().unwrap()).files.is_empty());
        assert!(commit(dir.to_str().unwrap(), "  ").is_err());

        let plain = tempfile::tempdir().unwrap();
        assert!(!changes(plain.path().to_str().unwrap()).git);
        assert!(commit(plain.path().to_str().unwrap(), "x").is_err());
        assert!(!prompt(plain.path().to_str().unwrap()).contains("Git:"));
    }

    #[test]
    fn reads_github_remotes() {
        assert_eq!(github_repo("https://github.com/acme/app.git").as_deref(), Some("acme/app"));
        assert_eq!(github_repo("git@github.com:acme/app.git").as_deref(), Some("acme/app"));
        assert_eq!(github_repo("ssh://git@github.com/acme/app").as_deref(), Some("acme/app"));
        assert_eq!(github_repo("https://gitlab.com/acme/app.git"), None);
        assert_eq!(branch_name("Simplify totals: use sum()!"), "ai/simplify-totals-use-sum");
    }

    #[test]
    fn finds_the_pull_request_page_on_other_hosts() {
        assert_eq!(remote_location("git@gitlab.com:group/sub/app.git"), Some(("gitlab.com".into(), "group/sub/app".into())));
        assert_eq!(remote_location("ssh://git@git.example.org:2222/team/app.git"), Some(("git.example.org".into(), "team/app".into())));
        assert_eq!(remote_location("https://user@bitbucket.org/team/app.git"), Some(("bitbucket.org".into(), "team/app".into())));
        assert_eq!(remote_location("/srv/git/app.git"), None);
        assert_eq!(
            compare_page("git@github.com:acme/app.git", "main", "ai/x").as_deref(),
            Some("https://github.com/acme/app/compare/main...ai/x?expand=1")
        );
        assert_eq!(
            compare_page("https://gitlab.example.com/group/app.git", "main", "ai/x").as_deref(),
            Some("https://gitlab.example.com/group/app/-/merge_requests/new?merge_request%5Bsource_branch%5D=ai/x&merge_request%5Btarget_branch%5D=main")
        );
        assert_eq!(
            compare_page("git@bitbucket.org:team/app.git", "main", "ai/x").as_deref(),
            Some("https://bitbucket.org/team/app/pull-requests/new?source=ai/x&dest=main")
        );
        assert_eq!(compare_page("https://codeberg.org/me/app.git", "main", "ai/x").as_deref(), Some("https://codeberg.org/me/app/compare/main...ai/x"));
        assert_eq!(compare_page("https://git.example.org/me/app.git", "main", "ai/x"), None);
        let gitea = "remote: \nremote: Create a new pull request for 'ai/x':\nremote:   https://git.example.org/me/app/compare/main...ai/x\nremote: \nTo git.example.org:me/app.git";
        assert_eq!(link_from_push(gitea).as_deref(), Some("https://git.example.org/me/app/compare/main...ai/x"));
        assert_eq!(link_from_push("To /srv/git/app.git\n * [new branch] ai/x -> ai/x"), None);
    }

    #[test]
    fn pushes_a_new_branch_for_a_pull_request() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        let work = tmp.path().join("work");
        sh(tmp.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        sh(tmp.path(), &["clone", "-q", remote.to_str().unwrap(), work.to_str().unwrap()]);
        sh(&work, &["config", "user.name", "t"]);
        sh(&work, &["config", "user.email", "t@t"]);
        sh(&work, &["switch", "-q", "-c", "main"]);
        crate::chat::at_rest::write(work.join("a.txt"), "one\n").unwrap();
        commit(work.to_str().unwrap(), "First").unwrap();
        sh(&work, &["push", "-q", "-u", "origin", "main"]);
        crate::chat::at_rest::write(work.join("a.txt"), "two\n").unwrap();
        commit(work.to_str().unwrap(), "Second").unwrap();

        // On the default branch: a branch is made from the title and pushed; a local remote has
        // no page to open.
        let pr = open_pull_request(work.to_str().unwrap(), "Use two").unwrap();
        assert_eq!(pr, PullRequest { branch: "ai/use-two".into(), url: None, via: "none".into() });

        // A host that prints a link while receiving a branch (as Gitea and GitLab do): that link
        // is the page to finish the pull request on.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hook = remote.join("hooks/post-receive");
            crate::chat::at_rest::write(&hook, "#!/bin/sh\necho\necho \"Create a new pull request for 'ai/use-three':\"\necho \"  https://git.example.org/me/app/compare/main...ai/use-three\"\n").unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            sh(&work, &["switch", "-q", "main"]);
            crate::chat::at_rest::write(work.join("a.txt"), "three\n").unwrap();
            commit(work.to_str().unwrap(), "Third").unwrap();
            let pr = open_pull_request(work.to_str().unwrap(), "Use three").unwrap();
            assert_eq!(pr.url.as_deref(), Some("https://git.example.org/me/app/compare/main...ai/use-three"));
            assert_eq!(pr.via, "browser");
        }
        let remote_branches = Command::new("git").arg("-C").arg(&remote).args(["branch", "--list"]).output().unwrap();
        assert!(String::from_utf8_lossy(&remote_branches.stdout).contains("ai/use-two"));
    }

    #[test]
    fn parses_renames() {
        let files = parse("0\t0\tsrc/{old.rs => new.rs}\n", "R100\tsrc/old.rs\tsrc/new.rs\n");
        assert_eq!(files[0].path, "src/new.rs");
        assert_eq!(files[0].status, "renamed");
    }

    #[test]
    fn worktrees_get_their_own_branch_and_folder() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let base = std::env::temp_dir().join(format!("lumo-wt-{}", std::process::id()));
        let repo = base.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |dir: &Path, args: &[&str]| assert!(Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().unwrap().success());
        run(&repo, &["init", "-q"]);
        crate::chat::at_rest::write(repo.join("a.txt"), "a").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-qm", "first"]);
        let (folder, branch) = create_worktree(repo.to_str().unwrap(), "Add a login page!").unwrap();
        assert_eq!(branch, "lumo/add-a-login-page");
        assert!(folder.ends_with("app-add-a-login-page"));
        assert!(Path::new(&folder).join("a.txt").exists());
        // The same name again gets a second folder and branch.
        let (second, branch2) = create_worktree(repo.to_str().unwrap(), "Add a login page").unwrap();
        assert_eq!(branch2, "lumo/add-a-login-page-2");
        assert!(second.ends_with("app-add-a-login-page-2"));
        assert!(create_worktree(base.to_str().unwrap(), "x").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

}
