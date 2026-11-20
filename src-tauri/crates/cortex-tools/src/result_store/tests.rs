use super::*;
fn store() -> ResultStore {
    let store = ResultStore::default();
    store.activate(true);
    store
}
fn reference(preview: &Value) -> String {
    let text = preview.as_str().unwrap();
    text.split("result_id=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}
#[test]
fn cache_eviction_is_bounded_and_explicit() {
    let store = ResultStore {
        capacity: 50_000,
        ..Default::default()
    };
    store.activate(true);
    let first = reference(
        &store
            .preview(store.scope(), "jira", json!("a".repeat(40_000)))
            .unwrap(),
    );
    let second = reference(
        &store
            .preview(store.scope(), "slack", json!("b".repeat(40_000)))
            .unwrap(),
    );
    assert!(store
        .read(store.scope(), &json!({"result_id":first}))
        .is_err());
    assert!(store
        .read(store.scope(), &json!({"result_id":second}))
        .is_ok());
    assert!(store.state.lock().unwrap().bytes <= 50_000);
    let uncached = store
        .preview(store.scope(), "jira", json!("c".repeat(60_000)))
        .unwrap();
    assert!(uncached.as_str().unwrap().contains("too large to retain"));
    assert!(!uncached.as_str().unwrap().contains("result_id="));
}
#[test]
fn mcp_normalization_preserves_errors_resources_and_numeric_literals() {
    let text = "{ \"id\":9007199254740993, \"s\":\"a  b\" }";
    let input = json!({"content":[{"type":"text","text":text}],"structuredContent":{"id":9007199254740993_u64,"s":"a  b"},"isError":true});
    let compact = normalize_mcp(input);
    assert_eq!(compact["isError"], true);
    assert_eq!(
        compact["content"][0]["text"],
        "{\"id\":9007199254740993,\"s\":\"a  b\"}"
    );
    assert!(compact.get("structuredContent").is_none());
    let distinct = json!({"content":[{"type":"text","text":"{\"id\":9007199254740993}"},{"type":"resource_link","uri":"https://example.com"}],"structuredContent":{"id":9007199254740992_u64}});
    assert_eq!(normalize_mcp(distinct.clone()), distinct);
    assert_eq!(minify_json("{\"id\":1, \"id\":2}"), "{\"id\":1,\"id\":2}");
}
#[test]
fn a_page_cannot_loop_at_a_multibyte_character() {
    let store = store();
    let id = reference(
        &store
            .preview(store.scope(), "jira", json!("🦀".repeat(10_000)))
            .unwrap(),
    );
    assert!(store
        .read(
            store.scope(),
            &json!({"result_id":id,"offset":1,"limit":513})
        )
        .is_err());
    assert!(store
        .read(store.scope(), &json!({"result_id":id,"offset":2}))
        .is_err());
}
#[test]
fn small_results_keep_the_existing_ipc_shape() {
    let store = store();
    let value = json!({"content":[{"type":"text","text":"hello"}],"isError":false});
    assert_eq!(
        store
            .preview(store.scope(), "mcp-slack", value.clone())
            .unwrap(),
        value
    );
}
#[test]
fn filesystem_images_survive_preview_limits_without_relaxing_other_output_limits() {
    let store = store();
    let image = json!({"type":"cortex_file_image","image_id":"image","path":"/photos/cat.png","mime_type":"image/png","data":"a".repeat(60_000)});
    let transfer = store
        .preview_with_transport(
            store.scope(),
            "filesystem",
            json!({
                "type":"cortex_file", "file_id":"image", "path":"/photos/cat.png", "data":"bytes"
            }),
            true,
        )
        .unwrap()["value"]
        .clone();
    assert_eq!(
        store
            .complete_file(
                store.scope(),
                &json!({
                    "result_id":transfer["result_id"], "value":image
                })
            )
            .unwrap(),
        image
    );
    let mcp = store
        .preview(
            store.scope(),
            "mcp-files",
            json!({"content":[{"type":"text","text":image.to_string()}]}),
        )
        .unwrap();
    assert!(mcp.as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    let mut oversized = image.clone();
    oversized["data"] = json!("a".repeat(7_000_000));
    assert!(store
        .preview(store.scope(), "filesystem", oversized)
        .unwrap()
        .is_string());
    let old_scope = store.scope();
    store.activate(false);
    assert!(store.preview(old_scope, "filesystem", image).is_err());
}

#[test]
fn converted_files_reuse_bounded_previews_and_account_scoped_paging() {
    let store = store();
    let file =
        json!({"type":"cortex_file","file_id":"file","path":"/data.csv","data":"a".repeat(60_000)});
    let transfer = store
        .preview_with_transport(store.scope(), "filesystem", file.clone(), true)
        .unwrap()["value"]
        .clone();
    assert_eq!(transfer["data"], file["data"]);
    assert!(store.state.lock().unwrap().bytes < 100);
    let pending = json!({"result_id":transfer["result_id"]});
    assert!(store.read(store.scope(), &pending).is_err());
    let converted =
        json!({"content":{"encoding":"utf-8","content":"converted text ".repeat(10_000)}});
    let preview = store
        .complete_file(
            store.scope(),
            &json!({"result_id":transfer["result_id"],"value":converted}),
        )
        .unwrap();
    assert!(preview.as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    let id = reference(&preview);
    assert_eq!(
        store
            .source_connector(store.scope(), &json!({"result_id":id}))
            .unwrap(),
        "filesystem"
    );
    assert!(store
        .read(store.scope(), &json!({"result_id":id}))
        .unwrap()
        .as_str()
        .unwrap()
        .contains("converted text"));
    assert!(store
        .complete_file(
            store.scope(),
            &json!({"result_id":transfer["result_id"],"value":"again"})
        )
        .is_err());
    let transfer = store
        .preview_with_transport(store.scope(), "filesystem", file.clone(), true)
        .unwrap()["value"]
        .clone();
    let old_scope = store.scope();
    store.activate(true);
    assert!(store
        .complete_file(
            old_scope,
            &json!({"result_id":transfer["result_id"],"value":"late output"})
        )
        .is_err());
    let plain = store
        .preview(store.scope(), "filesystem", file.clone())
        .unwrap();
    assert!(plain.as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    let ineligible = store
        .preview_with_transport(store.scope(), "filesystem", file, false)
        .unwrap();
    assert_eq!(ineligible["file_transport"], false);
    assert!(ineligible["value"].as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    assert!(store
        .state
        .lock()
        .unwrap()
        .entries
        .iter()
        .all(|entry| !entry.pending_file));
}

#[test]
fn mcp_screenshots_keep_image_bytes_separate_from_bounded_prose_and_preserve_source() {
    let store = store();
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(vec![42; 40_000]);
    let data = STANDARD.encode(&bytes);
    let result = json!({"content":[
            {"type":"text","text":"Screenshot captured".repeat(5000)},
            {"type":"image","mimeType":"image/png","data":data},
            {"type":"resource_link","uri":"https://example.com/page"}
        ],"structuredContent":{"title":"Page title"},"isError":true});
    let plain = store
        .preview(store.scope(), "mcp-chrome-devtools", result.clone())
        .unwrap();
    assert!(plain.as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    assert!(store
        .state
        .lock()
        .unwrap()
        .entries
        .iter()
        .all(|entry| !entry.pending_file));
    let transfer = store
        .preview_with_transport(store.scope(), "mcp-chrome-devtools", result, false)
        .unwrap()["value"]
        .clone();
    assert_eq!(transfer["type"], "cortex_tool_result");
    assert_eq!(transfer["isError"], true);
    assert_eq!(transfer["files"][0]["data"], data);
    assert!(transfer["text"].as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    assert!(!transfer["text"].as_str().unwrap().contains(&data));
    let text_id = reference(&transfer["text"]);
    assert_eq!(
        store
            .source_connector(store.scope(), &json!({"result_id":text_id}))
            .unwrap(),
        "mcp-chrome-devtools"
    );
    let receipt = &transfer["files"][0]["result_id"];
    assert_eq!(
        store
            .source_connector(store.scope(), &json!({"result_id":receipt}))
            .unwrap(),
        "mcp-chrome-devtools"
    );
    let processed = json!({"type":"cortex_file_image","image_id":"screenshot","path":"Chrome screenshot","mime_type":"image/png","data":data});
    assert_eq!(
        store
            .complete_file(
                store.scope(),
                &json!({"result_id":receipt,"value":processed})
            )
            .unwrap(),
        processed
    );
}

#[test]
fn invalid_mcp_images_do_not_hide_text_metadata_or_error_flags() {
    let store = store();
    let result = json!({"content":[{"type":"text","text":"capture failed"},
            {"type":"image","mimeType":"image/png","data":"invalid base64!"}],
            "structuredContent":{"status":"failed"},"isError":true});
    let transfer = store
        .preview_with_transport(store.scope(), "mcp-browser", result, false)
        .unwrap()["value"]
        .clone();
    assert_eq!(transfer["files"], json!([]));
    assert_eq!(transfer["text"]["content"][0]["text"], "capture failed");
    assert_eq!(transfer["text"]["structuredContent"]["status"], "failed");
    assert_eq!(transfer["text"]["isError"], true);
    assert!(transfer["text"].to_string().contains("invalid data"));
}

#[test]
fn embedded_mcp_image_resources_use_the_same_image_transfer() {
    let store = store();
    let data = STANDARD.encode(b"\x89PNG\r\n\x1a\nimage bytes");
    let result = json!({"content":[{"type":"resource","resource":{
        "uri":"screenshot://page/1","mimeType":"image/png","blob":data
    }}]});
    let transfer = store
        .preview_with_transport(store.scope(), "mcp-browser", result, false)
        .unwrap()["value"]
        .clone();
    assert_eq!(transfer["files"][0]["data"], data);
    assert_eq!(transfer["files"][0]["path"], "screenshot://page/1");
    assert!(transfer["text"].to_string().contains("screenshot://page/1"));
    assert!(!transfer["text"].to_string().contains(&data));
}
#[test]
fn oversized_mcp_output_is_bounded_and_retrievable_without_running_the_tool_again() {
    let store = store();
    let original = json!({"content":[{"type":"text","text":"🦀漢".repeat(20_000)}],"isError":true});
    let preview = store
        .preview(store.scope(), "mcp-slack", original.clone())
        .unwrap();
    assert!(preview.as_str().unwrap().len() <= MAX_PREVIEW_BYTES);
    assert!(preview.as_str().unwrap().contains("Do not repeat"));
    assert!(preview.as_str().unwrap().contains("isError"));
    let id = reference(&preview);
    let mut offset = 0;
    let mut reconstructed = String::new();
    loop {
        let page = store
            .read(
                store.scope(),
                &json!({"result_id":id,"offset":offset,"limit":999_999}),
            )
            .unwrap();
        let text = page.as_str().unwrap();
        assert!(text.len() <= MAX_PAGE_BYTES);
        let (header, body) = text.split_once('\n').unwrap();
        let meta: Value = serde_json::from_str(header).unwrap();
        reconstructed.push_str(body);
        if meta["next_offset"].is_null() {
            break;
        }
        offset = meta["next_offset"].as_u64().unwrap();
    }
    assert_eq!(reconstructed, original.to_string());
}
#[test]
fn account_switches_invalidate_references_and_reject_late_results_even_after_switching_back() {
    let store = store();
    let scope = store.scope();
    let preview = store
        .preview(scope, "jira", json!("x".repeat(40_000)))
        .unwrap();
    let id = reference(&preview);
    store.activate(false);
    store.activate(true);
    assert!(store.read(store.scope(), &json!({"result_id":id})).is_err());
    assert!(store
        .preview(scope, "jira", json!("late write result"))
        .is_err());
}
#[test]
fn poisoned_cache_is_cleared_without_panicking_in_result_methods() {
    let store = store();
    let old_scope = store.scope();
    let id = reference(
        &store
            .preview(old_scope, "jira", json!("x".repeat(40_000)))
            .unwrap(),
    );
    std::thread::scope(|threads| {
        let result = threads
            .spawn(|| {
                let _guard = store.state.lock().unwrap();
                panic!("poison the result cache");
            })
            .join();
        assert!(result.is_err());
    });

    let input = json!({"result_id": id});
    assert!(store.read(old_scope, &input).is_err());
    assert!(store.source_connector(old_scope, &input).is_err());
    assert!(store.preview(old_scope, "jira", json!("small")).is_err());
    assert!(store.is_active());
    let fresh_scope = store.scope();
    assert_ne!(fresh_scope, old_scope);
    assert!(store.read(fresh_scope, &input).is_err());
    let fresh_id = reference(
        &store
            .preview(fresh_scope, "jira", json!("y".repeat(40_000)))
            .unwrap(),
    );
    assert!(store
        .read(fresh_scope, &json!({"result_id": fresh_id}))
        .is_ok());
}
#[test]
fn invalid_offsets_and_unknown_references_are_errors() {
    let store = store();
    let preview = store
        .preview(store.scope(), "jira", json!("x".repeat(40_000)))
        .unwrap();
    let id = reference(&preview);
    assert!(store
        .read(store.scope(), &json!({"result_id":id,"offset":99_999}))
        .is_err());
    assert!(store
        .read(store.scope(), &json!({"result_id":"unknown"}))
        .is_err());
    assert!(store
        .read(store.scope(), &json!({"result_id":id,"limit":0}))
        .is_err());
}
