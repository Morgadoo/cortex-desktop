//! The registry with the app's own connectors (the registry's tests are in cortex-tools).

use crate::tools::error::ToolError;
use crate::tools::registry::ToolRegistry;
use serde_json::json;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

#[test]
fn read_only_filesystem_access_reads_but_cannot_create_or_edit_files() {
    use crate::tools::connectors::filesystem::FilesystemConnector;
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("existing.txt");
    let created = dir.path().join("created.txt");
    std::fs::write(&existing, "original").unwrap();
    let registry = ToolRegistry::new();
    registry.register(FilesystemConnector::new(
        vec![dir.path().to_path_buf()],
        true,
    ));
    assert!(block_on(registry.dispatch_with_access(
        "filesystem",
        "fs_read",
        json!({"path": existing}),
        true,
        true,
    ))
    .is_ok());
    for (name, input) in [
        ("fs_write", json!({"path": created, "content": "new"})),
        (
            "fs_edit",
            json!({"path": existing, "old_string": "original", "new_string": "changed"}),
        ),
    ] {
        assert!(matches!(
            block_on(registry.dispatch_with_access("filesystem", name, input, true, true,)),
            Err(ToolError::AccessDenied(_))
        ));
    }
    assert!(!created.exists());
    assert_eq!(std::fs::read_to_string(existing).unwrap(), "original");
}
