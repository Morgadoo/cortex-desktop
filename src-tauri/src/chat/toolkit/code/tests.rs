use super::*;

#[test]
fn pull_request_status_and_checks() {
    let gh = r#"{"number":42,"title":"Use two","url":"https://github.com/me/app/pull/42","state":"OPEN","isDraft":false,"reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[
        {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://x/1"},
        {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://x/2"},
        {"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":""},
        {"__typename":"StatusContext","context":"ci/legacy","state":"PENDING","targetUrl":"https://x/3"}]}"#;
    let pr = parse_gh_pr(gh).unwrap();
    assert_eq!(
        (pr.number, pr.state.as_str(), pr.review.as_str()),
        (42, "open", "changes requested")
    );
    let states: Vec<(&str, &str)> = pr
        .checks
        .iter()
        .map(|c| (c.name.as_str(), c.state.as_str()))
        .collect();
    assert_eq!(
        states,
        [
            ("test", "passed"),
            ("lint", "failed"),
            ("build", "running"),
            ("ci/legacy", "pending")
        ]
    );
    let mr = parse_glab_mr(r#"{"iid":7,"title":"T","web_url":"https://gitlab.com/me/app/-/merge_requests/7","state":"merged","head_pipeline":{"status":"success","web_url":"https://gl/p"}}"#).unwrap();
    assert_eq!(
        (mr.number, mr.state.as_str(), mr.checks[0].state.as_str()),
        (7, "merged", "passed")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("gh");
        std::fs::write(&fake, format!("#!/bin/sh\ncat <<'EOF'\n{gh}\nEOF\n")).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let got = pr_status_with(
            dir.path().to_str().unwrap(),
            fake.to_str().unwrap(),
            "/nonexistent/glab",
        )
        .unwrap();
        assert_eq!(got.checks.len(), 4);
        assert!(pr_status_with(
            dir.path().to_str().unwrap(),
            "/nonexistent/gh",
            "/nonexistent/glab"
        )
        .is_none());
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
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
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
    crate::chat::at_rest::write(dir.join("AGENTS.md"), "Run `make test` before finishing.")
        .unwrap();
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
    assert_eq!(
        c.files,
        vec![
            ChangedFile {
                path: "a.txt".into(),
                status: "modified".into(),
                additions: 2,
                deletions: 1
            },
            ChangedFile {
                path: "new.txt".into(),
                status: "untracked".into(),
                additions: 2,
                deletions: 0
            },
        ]
    );
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
    assert_eq!(
        github_repo("https://github.com/acme/app.git").as_deref(),
        Some("acme/app")
    );
    assert_eq!(
        github_repo("git@github.com:acme/app.git").as_deref(),
        Some("acme/app")
    );
    assert_eq!(
        github_repo("ssh://git@github.com/acme/app").as_deref(),
        Some("acme/app")
    );
    assert_eq!(github_repo("https://gitlab.com/acme/app.git"), None);
    assert_eq!(
        branch_name("Simplify totals: use sum()!"),
        "ai/simplify-totals-use-sum"
    );
}

#[test]
fn finds_the_pull_request_page_on_other_hosts() {
    assert_eq!(
        remote_location("git@gitlab.com:group/sub/app.git"),
        Some(("gitlab.com".into(), "group/sub/app".into()))
    );
    assert_eq!(
        remote_location("ssh://git@git.example.org:2222/team/app.git"),
        Some(("git.example.org".into(), "team/app".into()))
    );
    assert_eq!(
        remote_location("https://user@bitbucket.org/team/app.git"),
        Some(("bitbucket.org".into(), "team/app".into()))
    );
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
    assert_eq!(
        compare_page("https://codeberg.org/me/app.git", "main", "ai/x").as_deref(),
        Some("https://codeberg.org/me/app/compare/main...ai/x")
    );
    assert_eq!(
        compare_page("https://git.example.org/me/app.git", "main", "ai/x"),
        None
    );
    let gitea = "remote: \nremote: Create a new pull request for 'ai/x':\nremote:   https://git.example.org/me/app/compare/main...ai/x\nremote: \nTo git.example.org:me/app.git";
    assert_eq!(
        link_from_push(gitea).as_deref(),
        Some("https://git.example.org/me/app/compare/main...ai/x")
    );
    assert_eq!(
        link_from_push("To /srv/git/app.git\n * [new branch] ai/x -> ai/x"),
        None
    );
}

#[test]
fn pushes_a_new_branch_for_a_pull_request() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let work = tmp.path().join("work");
    sh(
        tmp.path(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    sh(
        tmp.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
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
    assert_eq!(
        pr,
        PullRequest {
            branch: "ai/use-two".into(),
            url: None,
            via: "none".into()
        }
    );

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
        assert_eq!(
            pr.url.as_deref(),
            Some("https://git.example.org/me/app/compare/main...ai/use-three")
        );
        assert_eq!(pr.via, "browser");
    }
    let remote_branches = Command::new("git")
        .arg("-C")
        .arg(&remote)
        .args(["branch", "--list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&remote_branches.stdout).contains("ai/use-two"));
}

#[test]
fn parses_renames() {
    let files = parse(
        "0\t0\tsrc/{old.rs => new.rs}\n",
        "R100\tsrc/old.rs\tsrc/new.rs\n",
    );
    assert_eq!(files[0].path, "src/new.rs");
    assert_eq!(files[0].status, "renamed");
}

#[test]
fn worktrees_get_their_own_branch_and_folder() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let base = std::env::temp_dir().join(format!("cortex-wt-{}", std::process::id()));
    let repo = base.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    let run = |dir: &Path, args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success())
    };
    run(&repo, &["init", "-q"]);
    crate::chat::at_rest::write(repo.join("a.txt"), "a").unwrap();
    run(&repo, &["add", "."]);
    run(&repo, &["commit", "-qm", "first"]);
    let (folder, branch) = create_worktree(repo.to_str().unwrap(), "Add a login page!").unwrap();
    assert_eq!(branch, "cortex/add-a-login-page");
    assert!(folder.ends_with("app-add-a-login-page"));
    assert!(Path::new(&folder).join("a.txt").exists());
    // The same name again gets a second folder and branch.
    let (second, branch2) = create_worktree(repo.to_str().unwrap(), "Add a login page").unwrap();
    assert_eq!(branch2, "cortex/add-a-login-page-2");
    assert!(second.ends_with("app-add-a-login-page-2"));
    assert!(create_worktree(base.to_str().unwrap(), "x").is_err());
    let _ = std::fs::remove_dir_all(&base);
}
