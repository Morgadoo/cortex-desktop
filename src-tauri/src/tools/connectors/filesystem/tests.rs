#[test]
fn overwrites_report_a_compact_diff() {
    let diff = super::compact_diff("a\nb\nc\n", "a\nB\nc\n");
    assert!(diff.contains("-b") && diff.contains("+B"), "{diff}");
    assert_eq!(super::compact_diff("x", "x"), "unchanged");
}

use super::*;
use serde_json::json;
use tempfile::{tempdir, TempDir};

/// The kernel's guardrail: the agent cannot change its evaluator, its permissions or its audit
/// log. They live in Cortex's own folder, which tools cannot reach even under an allowed root.
#[tokio::test]
async fn the_eval_sets_traces_and_audit_log_are_out_of_reach() {
    let home = tempdir().unwrap();
    let config = home.path().join(".config").join("cortex");
    for dir in ["evals/tool-selection/cases", "chat/audit"] {
        std::fs::create_dir_all(config.join(dir)).unwrap();
    }
    std::fs::write(config.join("chat/traces.db"), b"").unwrap();
    std::fs::write(config.join("connector_state.json"), b"{}").unwrap();
    let core = FilesystemCore::new(
        vec![home.path().to_path_buf()],
        vec![config.canonicalize().unwrap()],
        true,
    );
    for target in [
        "evals/tool-selection/cases/new.json",
        "chat/traces.db",
        "chat/audit/2026-11.jsonl",
        "connector_state.json",
    ] {
        let path = config.join(target);
        let written = core
            .fs_write(json!({ "path": path.to_string_lossy(), "content": "tampered" }))
            .await;
        assert!(
            matches!(&written, Err(ToolError::AccessDenied(m)) if m.contains("not available to tools")),
            "{target}: {written:?}"
        );
    }
    // Elsewhere under the same root, writing works.
    let ok = core
        .fs_write(
            json!({ "path": home.path().join("notes.txt").to_string_lossy(), "content": "x" }),
        )
        .await;
    assert!(ok.is_ok(), "{ok:?}");
}

fn core_rooted_at_temp_dir(write_enabled: bool) -> (TempDir, FilesystemCore) {
    let root = tempdir().unwrap();
    let core = FilesystemCore::new(vec![root.path().to_path_buf()], vec![], write_enabled);
    (root, core)
}

#[tokio::test]
async fn cortex_own_directories_stay_out_of_reach_inside_an_allowed_folder() {
    let root = tempdir().unwrap();
    let app_dir = root
        .path()
        .join("Application Support")
        .join("io.github.morgadoo.cortex-desktop");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("mcp_servers.json"), "{\"servers\":[]}").unwrap();
    std::fs::write(root.path().join("notes.txt"), "servers").unwrap();
    let core = FilesystemCore::new(vec![root.path().to_path_buf()], vec![app_dir.clone()], true);
    let config = app_dir
        .join("mcp_servers.json")
        .to_string_lossy()
        .into_owned();

    assert!(matches!(
        core.fs_read(json!({ "path": config })).await,
        Err(ToolError::AccessDenied(_))
    ));
    assert!(matches!(
        core.fs_write(json!({ "path": config, "content": "{}" }))
            .await,
        Err(ToolError::AccessDenied(_))
    ));
    let created = app_dir.join("new.json").to_string_lossy().into_owned();
    assert!(matches!(
        core.fs_write(json!({ "path": created, "content": "{}" }))
            .await,
        Err(ToolError::AccessDenied(_))
    ));

    for mode in ["name", "content"] {
        let found = core
            .fs_search(json!({ "query": "servers", "mode": mode }))
            .await
            .unwrap();
        let text = found.to_string();
        assert!(!text.contains("mcp_servers.json"), "{mode}: {text}");
    }
    assert!(core
        .fs_search(json!({ "query": "notes", "mode": "name" }))
        .await
        .unwrap()
        .to_string()
        .contains("notes.txt"));
}

fn create_file(root: &TempDir, file_name: &str, contents: &str) -> String {
    let path = root.path().join(file_name);
    std::fs::write(&path, contents).unwrap();
    path.to_string_lossy().into_owned()
}

fn path_in(root: &TempDir, file_name: &str) -> String {
    root.path().join(file_name).to_string_lossy().into_owned()
}

#[tokio::test]
async fn reading_a_file_returns_its_contents() {
    let (root, core) = core_rooted_at_temp_dir(false);
    let file = create_file(&root, "readable_document.txt", "hello world");
    let result = core.fs_read(json!({ "path": file })).await.unwrap();
    assert_eq!(result["content"]["content"], json!("hello world"));
}

#[tokio::test]
async fn whole_file_transport_is_format_agnostic_and_lossless() {
    let (root, core) = core_rooted_at_temp_dir(false);
    for (name, bytes) in [
        ("data.csv", b"name,value\nalice,1\n".to_vec()),
        ("sheet.xlsx", b"PK\x03\x04binary\0workbook".to_vec()),
        ("document.pdf", b"%PDF-1.4 opaque PDF bytes".to_vec()),
        ("document.docx", b"PK\x03\x04binary\0document".to_vec()),
        (
            "photo.data",
            [b"\x89PNG\r\n\x1a\n".as_slice(), &vec![42; 40_000]].concat(),
        ),
    ] {
        let path = root.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        let result = core
            .fs_read(json!({"path":path,"format":"file"}))
            .await
            .unwrap();
        assert_eq!(result["type"], "cortex_file");
        assert_eq!(result["name"], name);
        assert_eq!(result["size"], bytes.len());
        assert_eq!(
            STANDARD.decode(result["data"].as_str().unwrap()).unwrap(),
            bytes
        );
        assert!(core
            .fs_read(json!({"path":path,"format":"file","start":1}))
            .await
            .is_err());
    }
}

#[tokio::test]
async fn file_transport_enforces_size_and_folder_permissions() {
    let (root, core) = core_rooted_at_temp_dir(false);
    let path = root.path().join("large.csv");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_FILE_SIZE_BYTES + 1)
        .unwrap();
    assert!(core
        .fs_read(json!({"path":path,"format":"file"}))
        .await
        .unwrap_err()
        .to_string()
        .contains("5 MiB"));
    let outside = tempdir().unwrap();
    let path = outside.path().join("secret.pdf");
    std::fs::write(&path, "secret").unwrap();
    assert!(matches!(
        core.fs_read(json!({"path":path,"format":"file"})).await,
        Err(ToolError::AccessDenied(_))
    ));
}

#[test]
fn recognizes_supported_image_signatures() {
    for (header, mime) in [
        (b"\x89PNG\r\n\x1a\n".as_slice(), "image/png"),
        (b"\xff\xd8\xff".as_slice(), "image/jpeg"),
        (b"GIF89a".as_slice(), "image/gif"),
        (b"RIFF1234WEBP".as_slice(), "image/webp"),
        (b"BM".as_slice(), "image/bmp"),
    ] {
        assert_eq!(image_mime_type(header), Some(mime));
    }
    assert_eq!(image_mime_type(b"RIFF1234WAVE"), None);
    assert_eq!(image_mime_type(b"ordinary text"), None);
}

#[tokio::test]
async fn writing_a_new_file_under_the_root_creates_it_on_disk() {
    let (root, core) = core_rooted_at_temp_dir(true);
    let path = path_in(&root, "created_by_write.txt");
    core.fs_write(json!({ "path": path, "content": "fresh contents" }))
        .await
        .unwrap();
    let on_disk = std::fs::read_to_string(root.path().join("created_by_write.txt")).unwrap();
    assert_eq!(on_disk, "fresh contents");
}

#[tokio::test]
async fn writing_a_new_file_outside_the_root_is_denied() {
    let (_root, core) = core_rooted_at_temp_dir(true);
    let outside_root = tempdir().unwrap();
    let target = outside_root.path().join("created_outside_root.txt");
    let error = core
        .fs_write(json!({ "path": target.to_string_lossy(), "content": "escaped" }))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::AccessDenied(_)));
    assert!(!target.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn writing_a_new_file_through_a_symlink_that_escapes_the_root_is_denied() {
    let (root, core) = core_rooted_at_temp_dir(true);
    let outside_root = tempdir().unwrap();
    let link_inside_root = root.path().join("link_escaping_root.txt");
    std::os::unix::fs::symlink(outside_root.path().join("smuggled.txt"), &link_inside_root)
        .unwrap();
    let error = core
        .fs_write(json!({ "path": link_inside_root.to_string_lossy(), "content": "smuggled" }))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::AccessDenied(_)));
    assert!(!outside_root.path().join("smuggled.txt").exists());
}

#[tokio::test]
async fn writing_over_an_existing_file_replaces_its_contents() {
    let (root, core) = core_rooted_at_temp_dir(true);
    create_file(&root, "existing_document.txt", "old contents");
    core.fs_write(json!({
        "path": path_in(&root, "existing_document.txt"),
        "content": "new contents"
    }))
    .await
    .unwrap();
    let on_disk = std::fs::read_to_string(root.path().join("existing_document.txt")).unwrap();
    assert_eq!(on_disk, "new contents");
}

#[tokio::test]
async fn editing_a_file_replaces_matched_text_on_disk() {
    let (root, core) = core_rooted_at_temp_dir(true);
    create_file(&root, "editable_document.txt", "the quick brown fox");
    core.fs_edit(json!({
        "path": path_in(&root, "editable_document.txt"),
        "old_string": "quick",
        "new_string": "slow"
    }))
    .await
    .unwrap();
    let on_disk = std::fs::read_to_string(root.path().join("editable_document.txt")).unwrap();
    assert_eq!(on_disk, "the slow brown fox");
}

#[tokio::test]
async fn reading_a_parent_traversal_path_is_denied() {
    let (_root, core) = core_rooted_at_temp_dir(false);
    let error = core
        .fs_read(json!({ "path": "../outside_the_root.txt" }))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::AccessDenied(_)));
}

#[test]
fn a_match_after_text_that_grows_when_lowercased_is_found_in_the_original_line() {
    let line = format!("{}needle here", "İ".repeat(10));
    let start = find_ignoring_case(&line, "needle").unwrap();
    assert_eq!(&line[start..start + 6], "needle");
    assert_eq!(find_ignoring_case("Straße NEEDLE", "needle"), Some(8));
    assert_eq!(find_ignoring_case("ΟΔΟΣ", &lowercase("ΟΔΟΣ")), Some(0));
    assert_eq!(find_ignoring_case("nothing", "needle"), None);
}

#[test]
fn snippets_are_cut_on_character_boundaries() {
    for filler in ["é", "日本", "İ", "🦀"] {
        let line = format!(
            "{}needle{}",
            filler.repeat(CONTENT_CONTEXT_CHARS),
            filler.repeat(CONTENT_CONTEXT_CHARS)
        );
        let found = find_ignoring_case(&line, "needle").unwrap();
        assert!(snippet_around(&line, found).contains("needle"), "{filler}");
        for start in found.saturating_sub(CONTENT_CONTEXT_CHARS)..line.len() {
            snippet_around(&line, start);
        }
    }
    assert_eq!(snippet_around("", 0), "");
}

#[tokio::test]
async fn content_search_over_non_ascii_and_non_utf8_files_returns_matches() {
    let (root, core) = core_rooted_at_temp_dir(false);
    let accented = format!("{}NEEDLE{}", "İé日".repeat(200), "🦀".repeat(200));
    create_file(&root, "accented.txt", &accented);
    std::fs::write(root.path().join("latin1.txt"), b"caf\xe9 needle\n").unwrap();

    let found = core
        .fs_search(json!({ "query": "needle", "mode": "content" }))
        .await
        .unwrap()
        .to_string();
    assert!(found.contains("accented.txt"), "{found}");
    assert!(found.contains("NEEDLE"), "{found}");
}

const SHARES_AND_DEVICES: [&str; 7] = [
    r"\\attacker.example\share\x",
    "//attacker.example/share/x",
    r"\/attacker.example/share/x",
    r"\\?\UNC\attacker.example\share\x",
    r"\\.\pipe\x",
    r"\\?\GLOBALROOT\Device\Mup\attacker.example\share\x",
    r"\\attacker.example\share",
];

fn refused_as_remote(result: Result<impl std::fmt::Debug, ToolError>) -> bool {
    matches!(result, Err(ToolError::AccessDenied(message)) if message.contains("network"))
}

#[tokio::test]
async fn network_and_device_paths_are_refused_before_the_os_sees_them() {
    let (_root, core) = core_rooted_at_temp_dir(true);
    for path in SHARES_AND_DEVICES {
        assert!(refused_as_remote(sanitize_user_path(path)), "{path}");
        assert!(
            refused_as_remote(core.fs_read(json!({ "path": path })).await),
            "{path}"
        );
        assert!(
            refused_as_remote(core.fs_write(json!({ "path": path, "content": "x" })).await),
            "{path}"
        );
        assert!(
            refused_as_remote(
                core.fs_edit(json!({ "path": path, "old_string": "a", "new_string": "b" }))
                    .await
            ),
            "{path}"
        );
        assert!(
            refused_as_remote(core.fs_search(json!({ "query": "x", "root": path })).await),
            "{path}"
        );
    }
}

#[test]
fn local_paths_still_pass_the_path_check() {
    assert!(sanitize_user_path("notes/today.txt").is_ok());
    assert!(sanitize_user_path("/Users/me/notes.txt").is_ok());
    if cfg!(windows) {
        assert!(sanitize_user_path(r"C:\Users\me\notes.txt").is_ok());
        assert!(sanitize_user_path(r"\\?\C:\Users\me\notes.txt").is_ok());
    }
}

#[tokio::test]
async fn reading_an_absolute_path_outside_the_root_is_denied() {
    let (_root, core) = core_rooted_at_temp_dir(false);
    let outside_root = tempdir().unwrap();
    let secret = outside_root.path().join("secret_outside_root.txt");
    std::fs::write(&secret, "confidential").unwrap();
    let error = core
        .fs_read(json!({ "path": secret.to_string_lossy() }))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::AccessDenied(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn reading_through_a_symlink_that_escapes_the_root_is_denied() {
    let (root, core) = core_rooted_at_temp_dir(false);
    let outside_root = tempdir().unwrap();
    let secret = outside_root.path().join("secret_outside_root.txt");
    std::fs::write(&secret, "confidential").unwrap();
    let link_inside_root = root.path().join("link_escaping_root.txt");
    std::os::unix::fs::symlink(&secret, &link_inside_root).unwrap();
    let error = core
        .fs_read(json!({ "path": link_inside_root.to_string_lossy() }))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::AccessDenied(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn content_search_skips_symlinks_that_escape_the_root() {
    let (root, core) = core_rooted_at_temp_dir(false);
    let outside_root = tempdir().unwrap();
    let secret = outside_root.path().join("secret_outside_root.txt");
    std::fs::write(&secret, "needle confidential").unwrap();
    std::os::unix::fs::symlink(&secret, root.path().join("link_escaping_root.txt")).unwrap();
    create_file(&root, "inside_root.txt", "needle visible");
    let result = core
        .fs_search(json!({ "query": "needle", "mode": "content" }))
        .await
        .unwrap()
        .to_string();
    assert!(result.contains("visible"));
    assert!(!result.contains("confidential"));
}

#[tokio::test]
async fn writing_is_denied_when_write_access_is_disabled() {
    let (root, core) = core_rooted_at_temp_dir(false);
    let error = core
        .fs_write(json!({
            "path": path_in(&root, "should_not_be_created.txt"),
            "content": "blocked"
        }))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::AccessDenied(_)));
    assert!(!root.path().join("should_not_be_created.txt").exists());
}
