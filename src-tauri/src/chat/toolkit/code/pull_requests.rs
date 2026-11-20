//! Pull and merge requests: their status from gh or glab, compare pages, and opening one.

use super::*;
use crate::chat::error::ChatError;

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
pub(super) fn check_state(v: &serde_json::Value) -> String {
    let s = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_uppercase()
    };
    match (
        s("status").as_str(),
        s("conclusion").as_str(),
        s("state").as_str(),
    ) {
        (_, "SUCCESS", _) | (_, _, "SUCCESS") => "passed",
        (_, "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE", _)
        | (_, _, "FAILURE" | "ERROR") => "failed",
        (_, "SKIPPED" | "NEUTRAL", _) => "skipped",
        ("IN_PROGRESS" | "QUEUED" | "WAITING" | "PENDING" | "REQUESTED", _, _) => "running",
        (_, _, "PENDING" | "EXPECTED") => "pending",
        _ => "pending",
    }
    .into()
}

/// Reads `gh pr view --json …` output.
pub(in crate::chat::toolkit) fn parse_gh_pr(json: &str) -> Option<PrStatus> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let state = match (
        v["state"].as_str().unwrap_or_default(),
        v["isDraft"].as_bool().unwrap_or(false),
    ) {
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
            name: c["name"]
                .as_str()
                .or(c["context"].as_str())
                .unwrap_or("check")
                .to_string(),
            state: check_state(c),
            url: c["detailsUrl"]
                .as_str()
                .or(c["targetUrl"].as_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    Some(PrStatus {
        number: v["number"].as_u64()?,
        title: v["title"].as_str().unwrap_or_default().into(),
        url: v["url"].as_str().unwrap_or_default().into(),
        state: state.into(),
        review: review.into(),
        checks,
    })
}

/// Reads `glab mr view -F json` output.
pub(in crate::chat::toolkit) fn parse_glab_mr(json: &str) -> Option<PrStatus> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let state = match (
        v["state"].as_str().unwrap_or_default(),
        v["draft"].as_bool().unwrap_or(false),
    ) {
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
        vec![Check {
            name: "Pipeline".into(),
            state: st.into(),
            url: pipeline["web_url"].as_str().unwrap_or_default().into(),
        }]
    } else {
        Vec::new()
    };
    Some(PrStatus {
        number: v["iid"].as_u64()?,
        title: v["title"].as_str().unwrap_or_default().into(),
        url: v["web_url"].as_str().unwrap_or_default().into(),
        state: state.into(),
        review: if v["approved"].as_bool() == Some(true) {
            "approved".into()
        } else {
            String::new()
        },
        checks,
    })
}

/// The pull request of the folder's current branch, through `gh` or `glab`; `None` when there is
/// none or neither tool is installed and signed in. `tools` names the programs (tests swap them).
pub(in crate::chat::toolkit) fn pr_status_with(
    dir: &str,
    gh: &str,
    glab: &str,
) -> Option<PrStatus> {
    let out = Command::new(gh)
        .current_dir(dir)
        .args([
            "pr",
            "view",
            "--json",
            "number,title,url,state,isDraft,reviewDecision,statusCheckRollup",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success());
    if let Some(pr) = out.and_then(|o| parse_gh_pr(&String::from_utf8_lossy(&o.stdout))) {
        return Some(pr);
    }
    let out = Command::new(glab)
        .current_dir(dir)
        .args(["mr", "view", "-F", "json"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse_glab_mr(&String::from_utf8_lossy(&out.stdout))
}

pub(in crate::chat) fn pr_status(dir: &str) -> Option<PrStatus> {
    pr_status_with(dir, "gh", "glab")
}

/// `owner/repo` of a GitHub remote URL (https or ssh), if it is one.
pub(in crate::chat::toolkit) fn github_repo(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("http://github.com/"))
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty() && parts.next().is_none())
        .then(|| format!("{owner}/{repo}"))
}

/// Host and repository path of a remote URL (https, ssh or scp-style), e.g.
/// `("gitlab.com", "group/sub/app")`.
pub(in crate::chat::toolkit) fn remote_location(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = if let Some(r) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
        .or_else(|| remote.strip_prefix("ssh://"))
    {
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
pub(in crate::chat::toolkit) fn compare_page(
    remote: &str,
    base: &str,
    branch: &str,
) -> Option<String> {
    let (host, path) = remote_location(remote)?;
    let enc = |s: &str| {
        s.replace('%', "%25")
            .replace(' ', "%20")
            .replace('#', "%23")
            .replace('&', "%26")
            .replace('?', "%3F")
    };
    if host == "github.com" {
        return github_repo(remote).map(|repo| {
            format!(
                "https://github.com/{repo}/compare/{}...{}?expand=1",
                enc(base),
                enc(branch)
            )
        });
    }
    if host.contains("gitlab") {
        return Some(format!(
            "https://{host}/{path}/-/merge_requests/new?merge_request%5Bsource_branch%5D={}&merge_request%5Btarget_branch%5D={}",
            enc(branch),
            enc(base)
        ));
    }
    if host == "bitbucket.org" {
        return Some(format!(
            "https://bitbucket.org/{path}/pull-requests/new?source={}&dest={}",
            enc(branch),
            enc(base)
        ));
    }
    if host == "codeberg.org" || host.contains("gitea") || host.contains("forgejo") {
        return Some(format!(
            "https://{host}/{path}/compare/{}...{}",
            enc(base),
            enc(branch)
        ));
    }
    None
}

/// The link a host prints while pushing a new branch ("To create a merge request…"): GitLab,
/// Gitea, Forgejo and Bitbucket do this, self-hosted ones included.
pub(in crate::chat::toolkit) fn link_from_push(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim_start_matches("remote:").trim();
        let start = line.find("https://").or_else(|| line.find("http://"))?;
        let url = line[start..].split_whitespace().next()?;
        [
            "merge_requests/new",
            "pull-requests/new",
            "/compare/",
            "/pull/new/",
            "/pulls/new",
        ]
        .iter()
        .any(|m| url.contains(m))
        .then(|| url.to_string())
    })
}

/// A branch name from a title: `ai/` plus lowercase words joined by hyphens.
pub(super) fn branch_name(title: &str) -> String {
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
    format!(
        "ai/{}",
        if slug.is_empty() {
            "changes".into()
        } else {
            slug
        }
    )
}

/// Pushes the current branch (creating one first when on the default branch) and opens a pull
/// request for it: with the GitHub CLI (or GitLab's `glab`) when it is installed and signed in,
/// otherwise by handing back the host's new-pull-request page, where the user finishes it. The
/// page comes from the remote for GitHub, GitLab, Bitbucket, Codeberg, Gitea and Forgejo, or from
/// the link the host printed during the push.
pub(in crate::chat) fn open_pull_request(dir: &str, title: &str) -> Result<PullRequest, ChatError> {
    let path = Path::new(dir);
    let run = |args: &[&str]| -> Result<String, ChatError> {
        let out = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .map_err(ChatError::internal)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(ChatError::Invalid(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    };
    let remote = run(&["remote", "get-url", "origin"])
        .map_err(|_| "This repository has no `origin` remote to push to.".to_string())
        .map_err(ChatError::Invalid)?;
    let default = run(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .and_then(|r| r.strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| "main".into());
    let mut branch = run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == default || branch == "HEAD" {
        branch = branch_name(title);
        run(&["switch", "-c", &branch])?;
    }
    let push = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["push", "-u", "origin", &branch])
        .env("LC_ALL", "C")
        .output()
        .map_err(ChatError::internal)?;
    let push_output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&push.stdout),
        String::from_utf8_lossy(&push.stderr)
    );
    if !push.status.success() {
        return Err(ChatError::Invalid(format!(
            "Pushing {branch} failed: {}",
            String::from_utf8_lossy(&push.stderr).trim()
        )));
    }

    let repo = github_repo(&remote);
    // The GitHub CLI, when it is installed and signed in, opens the pull request directly.
    if repo.is_some() {
        let mut gh = Command::new("gh");
        gh.current_dir(path).args([
            "pr",
            "create",
            "--head",
            &branch,
            "--base",
            &default,
            "--title",
            title.trim(),
            "--fill",
        ]);
        if let Ok(out) = gh.output() {
            if out.status.success() {
                let url = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .rev()
                    .find(|l| l.starts_with("https://"))
                    .map(str::to_string);
                return Ok(PullRequest {
                    branch,
                    url,
                    via: "gh".into(),
                });
            }
        }
    }
    // GitLab's CLI does the same for GitLab remotes.
    if remote_location(&remote).is_some_and(|(host, _)| host.contains("gitlab")) {
        let mut glab = Command::new("glab");
        glab.current_dir(path).args([
            "mr",
            "create",
            "--source-branch",
            &branch,
            "--target-branch",
            &default,
            "--title",
            title.trim(),
            "--fill",
            "--yes",
        ]);
        if let Ok(out) = glab.output() {
            if out.status.success() {
                let url = String::from_utf8_lossy(&out.stdout)
                    .split_whitespace()
                    .rev()
                    .find(|w| w.starts_with("https://"))
                    .map(str::to_string);
                return Ok(PullRequest {
                    branch,
                    url,
                    via: "cli".into(),
                });
            }
        }
    }
    let url = compare_page(&remote, &default, &branch).or_else(|| link_from_push(&push_output));
    Ok(PullRequest {
        via: if url.is_some() { "browser" } else { "none" }.into(),
        url,
        branch,
    })
}
