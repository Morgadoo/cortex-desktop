//! Version history with the stores that keep versions: memories, instructions, notebooks and
//! knowledge pages, reverted and compared through `history`. The history core is
//! `cortex_store::versions`; these need the app's modules.

use crate::chat::versions::{keep, list};
use serde_json::json;

#[test]
fn a_memory_change_can_be_reverted() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let mut memories = crate::chat::memory::load(d);
    crate::chat::memory::add(&mut memories, "The user prefers metric units.", "chat").unwrap();
    crate::chat::memory::save(d, &memories).unwrap();
    let before = crate::chat::memory::load(d);
    crate::chat::memory::add(&mut memories, "The user lives in Lisbon.", "chat").unwrap();
    crate::chat::memory::save(d, &memories).unwrap();
    assert_ne!(crate::chat::memory::load(d), before);

    let newest = list(d, "memories", "all").unwrap().remove(0);
    crate::chat::history::restore(d, "memories", "all", &newest.id).unwrap();
    assert_eq!(crate::chat::memory::load(d), before);
    // The version the revert replaced is kept, so the revert can be undone too.
    let undo = list(d, "memories", "all").unwrap().remove(0);
    crate::chat::history::restore(d, "memories", "all", &undo.id).unwrap();
    assert_eq!(crate::chat::memory::load(d), memories);
    assert!(crate::chat::history::restore(d, "memories", "all", "nope").is_err());
}

#[test]
fn pages_instructions_and_notebooks_can_be_reverted() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let mut first = crate::chat::store::load_instructions(d);
    first.name = "Ana".into();
    crate::chat::store::save_instructions(d, &first).unwrap();
    let mut second = first.clone();
    second.name = "Rui".into();
    crate::chat::store::save_instructions(d, &second).unwrap();
    let v = list(d, "instructions", "user").unwrap().remove(0);
    crate::chat::history::restore(d, "instructions", "user", &v.id).unwrap();
    assert_eq!(crate::chat::store::load_instructions(d).name, "Ana");

    crate::chat::agents::save_notebook(d, "a1", "first notes").unwrap();
    crate::chat::agents::save_notebook(d, "a1", "second notes").unwrap();
    let v = list(d, "notebooks", "a1").unwrap().remove(0);
    crate::chat::history::restore(d, "notebooks", "a1", &v.id).unwrap();
    assert_eq!(crate::chat::agents::notebook(d, "a1"), "first notes");

    crate::chat::knowledge::save_all(d, &[]).unwrap();
    let page: crate::chat::knowledge::Page = serde_json::from_value(json!({
        "id": "p1", "title": "T", "body": "B", "author": "user",
        "updatedAt": "2026-11-01T00:00:00Z"
    }))
    .unwrap();
    crate::chat::knowledge::save_all(d, std::slice::from_ref(&page)).unwrap();
    let v = list(d, "knowledge", "pages").unwrap().remove(0);
    crate::chat::history::restore(d, "knowledge", "pages", &v.id).unwrap();
    assert!(crate::chat::knowledge::load(d).is_empty());
}

#[test]
fn a_version_diffs_against_the_one_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let one = serde_json::json!({ "text": "first", "n": 1 });
    let two = serde_json::json!({ "text": "second", "n": 1 });
    let three = serde_json::json!({ "text": "third", "n": 1 });
    keep(d, "memories", "all", &one, &two).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    keep(d, "memories", "all", &two, &three).unwrap();
    let versions = list(d, "memories", "all").unwrap();
    let oldest = &versions[1].id;
    let text = crate::chat::history::diff(d, "memories", "all", oldest, None).unwrap();
    assert!(
        text.contains("-   \"text\": \"first\"") && text.contains("+   \"text\": \"second\""),
        "{text}"
    );
    let newest = &versions[0].id;
    assert!(
        crate::chat::history::diff(d, "memories", "all", newest, Some(&three))
            .unwrap()
            .contains("third")
    );
    // Without `current`, the newest version is compared with the item as it is now.
    let to_now = crate::chat::history::diff(d, "memories", "all", newest, None).unwrap();
    assert!(to_now.contains("-   \"text\": \"second\""), "{to_now}");
    assert!(crate::chat::history::diff(d, "secrets", "all", newest, None).is_err());
}
