#[test]
fn an_exchange_is_deleted_with_its_reply() {
    let mut c = Conversation::new("c".into(), None, None);
    c.messages = vec![
        Message::user("one"),
        Message::assistant("1"),
        Message::user("two"),
        Message::assistant("2"),
        Message::user("three"),
        Message::assistant("3"),
    ];
    let now = Utc::now();
    c.times = vec![(0, now), (1, now), (2, now), (3, now), (4, now), (5, now)];
    c.ratings = vec![(3, 1), (5, -1)];
    delete_exchange(&mut c, 2).unwrap();
    assert_eq!(c.messages.len(), 4);
    assert_eq!(
        cortex_kernel::context::message_text(&c.messages[2]),
        "three"
    );
    assert_eq!(
        c.ratings,
        vec![(3, -1)],
        "the deleted reply's rating goes, later ones move back"
    );
    assert_eq!(
        c.times.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert!(
        delete_exchange(&mut c, 1).is_err(),
        "only typed messages start an exchange"
    );
    // The last exchange can go too.
    delete_exchange(&mut c, 2).unwrap();
    assert_eq!(c.messages.len(), 2);
}

#[test]
fn messages_are_rewritten_without_a_new_reply() {
    use rig_core::completion::message::AssistantContent;
    let mut c = Conversation::new("c".into(), None, None);
    let tool_turn = Message::Assistant(rig_core::completion::message::AssistantMessage::new(vec![
        AssistantContent::tool_call(
            "t1",
            rig_core::completion::message::ToolName::new("web_search").unwrap(),
            serde_json::json!({ "query": "x" }),
        ),
    ]));
    c.messages = vec![
        Message::user("what is 2+2?"),
        tool_turn,
        Message::assistant("five"),
        Message::user("thanks"),
    ];
    rewrite_message(&mut c, 0, "what is 2+3?").unwrap();
    assert_eq!(
        cortex_kernel::context::message_text(&c.messages[0]),
        "what is 2+3?"
    );
    // Editing an answer changes its final text and keeps the tool step before it.
    rewrite_message(&mut c, 2, "Four.").unwrap();
    assert_eq!(
        cortex_kernel::context::message_text(&c.messages[2]),
        "Four."
    );
    assert!(
        matches!(&c.messages[1], Message::Assistant(a) if matches!(a.content[0], AssistantContent::ToolCall(_)))
    );
    assert_eq!(c.messages.len(), 4);
    assert!(rewrite_message(&mut c, 3, "  ").is_err());
    assert!(rewrite_message(&mut c, 9, "x").is_err());
}

#[test]
fn bookmarks_keep_a_snippet_and_follow_deletions() {
    let mut c = Conversation::new("c".into(), None, None);
    c.messages = vec![
        Message::user("one"),
        Message::assistant("**First** answer"),
        Message::user("two"),
        Message::assistant("Second answer"),
    ];
    set_bookmark(&mut c, 3, true).unwrap();
    set_bookmark(&mut c, 1, true).unwrap();
    assert_eq!(
        c.bookmarks,
        vec![
            (1, "First answer".to_string()),
            (3, "Second answer".to_string())
        ]
    );
    assert!(set_bookmark(&mut c, 7, true).is_err());
    delete_exchange(&mut c, 0).unwrap();
    assert_eq!(
        c.bookmarks,
        vec![(1, "Second answer".to_string())],
        "the deleted answer's bookmark goes, the later one moves back"
    );
    set_bookmark(&mut c, 1, false).unwrap();
    assert!(c.bookmarks.is_empty());
}

#[test]
fn branches_keep_what_was_asked_for() {
    let mut c = Conversation::new("c".into(), None, None);
    c.messages = vec![
        Message::user(
            "Summarise this\n\n<attachment name=\"notes.txt\">\nsecret plans\n</attachment>",
        ),
        Message::assistant("A summary."),
        Message::user("two"),
        Message::assistant("2"),
    ];
    c.ratings = vec![(1, 1), (3, -1)];
    c.bookmarks = vec![(3, "2".into())];
    let mut upto = c.clone();
    branch_copy(&mut upto, Some(1), false).unwrap();
    assert_eq!(upto.messages.len(), 2);
    assert_eq!(
        (upto.ratings.clone(), upto.bookmarks.len()),
        (vec![(1, 1)], 0),
        "notes on later messages go with them"
    );
    assert!(cortex_kernel::context::message_text(&upto.messages[0]).contains("secret plans"));
    let mut bare = c.clone();
    branch_copy(&mut bare, None, true).unwrap();
    assert_eq!(bare.messages.len(), 4);
    assert_eq!(
        cortex_kernel::context::message_text(&bare.messages[0]),
        "Summarise this"
    );
    assert!(branch_copy(&mut c.clone(), Some(9), false).is_err());
}

#[test]
fn chat_settings_fall_back_to_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(chat_defaults(dir.path()), ChatParams::default());
    let saved = save_chat_defaults(
        dir.path(),
        ChatParams {
            temperature: Some(5.0),
            top_p: Some(0.9),
            instructions: "x".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        (saved.temperature, saved.top_p, saved.instructions.as_str()),
        (Some(2.0), Some(0.9), ""),
        "clamped, and no instructions in defaults"
    );
    let own = ChatParams {
        temperature: Some(0.2),
        ..Default::default()
    };
    let effective = own.or_defaults(&chat_defaults(dir.path()));
    assert_eq!(
        (effective.temperature, effective.top_p),
        (Some(0.2), Some(0.9)),
        "a chat's own value wins; unset ones come from the defaults"
    );
}

#[test]
fn tags_are_cleaned() {
    let tags = clean_tags(&["#Work".into(), " work ".into(), "".into(), "Q4 plan".into()]);
    assert_eq!(tags, ["work", "q4 plan"]);
}

#[test]
fn regenerated_answers_are_kept_as_versions() {
    let mut c = Conversation::new("v".into(), None, None);
    c.messages.push(Message::user("Q"));
    c.messages.push(Message::assistant("A1"));
    let text = |c: &Conversation| match c.messages.last() {
        Some(Message::Assistant(a)) => match a.content.first() {
            Some(rig_core::completion::message::AssistantContent::Text(t)) => t.text.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    // Regenerate twice: keep the answer, then a new one is written.
    keep_branch(&mut c, 0, "answer");
    c.messages.truncate(1);
    c.messages.push(Message::assistant("A2"));
    keep_branch(&mut c, 0, "answer");
    c.messages.truncate(1);
    c.messages.push(Message::assistant("A3"));
    let v = view(&c).versions[0].clone();
    assert_eq!((v.current, v.total), (2, 3));
    select_version(&mut c, 0, 0).unwrap();
    assert_eq!(text(&c), "A1");
    select_version(&mut c, 0, 1).unwrap();
    assert_eq!(text(&c), "A2");
    assert!(select_version(&mut c, 0, 7).is_err());
    assert_eq!(text(&c), "A2", "a bad index leaves the answer as it was");
    // Every version's answer, in switcher order, to compare.
    let from = view(&c).versions[0].from;
    assert_eq!(version_texts(&c, from).unwrap(), ["A1", "A2", "A3"]);
    // Versions saved in the old single-point format compare the same way.
    let mut old = c.clone();
    old.branches.clear();
    old.messages = vec![Message::user("Q"), Message::assistant("B2")];
    old.versions = Some(Versions {
        from: 0,
        others: vec![vec![Message::assistant("B1")]],
        current: 1,
        ..Default::default()
    });
    let from = view(&old).versions[0].from;
    assert_eq!(version_texts(&old, from).unwrap(), ["B1", "B2"]);
    select_version(&mut c, 0, 2).unwrap();
    assert_eq!(text(&c), "A3");
    // The branch point survives later messages; they belong to the branch showing.
    c.messages.push(Message::user("Next"));
    c.messages.push(Message::assistant("N1"));
    select_version(&mut c, 0, 0).unwrap();
    assert_eq!(c.messages.len(), 2, "branch A1 had no follow-up");
    select_version(&mut c, 0, 2).unwrap();
    assert_eq!(text(&c), "N1", "the follow-up came back with its branch");

    // Editing an earlier message branches there and keeps the old continuation.
    keep_branch(&mut c, 2, "prompt");
    c.messages.truncate(2);
    c.messages.push(Message::user("Next, edited"));
    c.messages.push(Message::assistant("E1"));
    // Both branch points are live now: the answer to Q, and the edited follow-up.
    let views = view(&c).versions;
    assert_eq!(views.len(), 2);
    let v = &views[1];
    assert_eq!(
        (v.from, v.kind.as_str(), v.current, v.total),
        (2, "prompt", 1, 2)
    );
    select_version(&mut c, 2, 0).unwrap();
    assert_eq!(text(&c), "N1");
    select_version(&mut c, 2, 1).unwrap();
    assert_eq!(text(&c), "E1");

    // A tree: switching the first answer away stores the edit branch inside it, and
    // switching back brings both the follow-up and its own branch point back.
    select_version(&mut c, 0, 0).unwrap();
    assert_eq!(c.messages.len(), 2);
    assert_eq!(
        view(&c).versions.len(),
        1,
        "the inner branch point left with its branch"
    );
    select_version(&mut c, 0, 2).unwrap();
    assert_eq!(text(&c), "E1");
    let views = view(&c).versions;
    assert_eq!(views.len(), 2, "the inner branch point came back");
    select_version(&mut c, 2, 0).unwrap();
    assert_eq!(text(&c), "N1");
}

#[test]
fn old_single_branch_points_still_load() {
    let mut c = Conversation::new("o".into(), None, None);
    c.messages.push(Message::user("Q"));
    c.messages.push(Message::assistant("A2"));
    // Saved before branching: the branch held answers after prompt 0.
    c.versions = Some(Versions {
        from: 0,
        kind: String::new(),
        others: vec![vec![Message::assistant("A1")]],
        nested: vec![],
        current: 1,
    });
    let v = view(&c).versions;
    assert_eq!((v[0].from, v[0].total), (1, 2));
    select_version(&mut c, 1, 0).unwrap();
    assert_eq!(c.messages.len(), 2);
    assert!(c.versions.is_none() && c.branches.len() == 1);
}

use super::*;
use rig_core::completion::message::{AssistantMessage, CallId, ToolName};

fn tool_call_message(id: &str, name: &str) -> Message {
    Message::Assistant(AssistantMessage::new(vec![AssistantContent::tool_call(
        id,
        ToolName::new(name).unwrap(),
        serde_json::json!({ "path": "/tmp" }),
    )]))
}

#[test]
fn ids_must_be_safe_file_names() {
    assert!(valid_id("8f2c-11aa_x"));
    assert!(!valid_id("../etc"));
    assert!(!valid_id(""));
    assert!(!valid_id("a/b"));
}

#[test]
fn conversations_round_trip_and_list_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut older = Conversation::new("a".into(), None, None);
    older.messages.push(Message::user("hi"));
    older.updated_at = Utc::now() - chrono::Duration::hours(1);
    save_conversation(dir.path(), &older).unwrap();
    let mut newer = Conversation::new("b".into(), None, None);
    newer.title = "Second".into();
    save_conversation(dir.path(), &newer).unwrap();

    let list = list_conversations(dir.path());
    assert_eq!(
        list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(list[1].title, "New chat");
    let loaded = load_conversation(dir.path(), "a").unwrap();
    assert_eq!(loaded.messages, older.messages);

    delete_conversation(dir.path(), "a").unwrap();
    assert_eq!(list_conversations(dir.path()).len(), 1);
}

#[test]
fn tool_results_fold_into_the_assistant_turn() {
    let mut c = Conversation::new("c".into(), None, None);
    c.messages.push(Message::user("list files"));
    c.messages
        .push(tool_call_message("call_1", "filesystem__list"));
    c.messages.push(Message::tool_result(
        CallId::from_wire("call_1"),
        ToolName::new("filesystem__list").unwrap(),
        "a.txt",
    ));
    c.messages.push(Message::assistant("There is one file."));

    let turns = turns(&c);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[1].role, "assistant");
    assert_eq!(turns[1].index, 3);
    match &turns[1].parts[0] {
        Part::Tool { name, output, .. } => {
            assert_eq!(name, "filesystem__list");
            assert_eq!(output.as_deref(), Some("a.txt"));
        }
        other => panic!("expected a tool part, got {other:?}"),
    }
    assert_eq!(
        turns[1].parts[1],
        Part::Text {
            text: "There is one file.".into()
        }
    );
}

#[test]
fn deleting_exchanges_keeps_the_rest_in_place() {
    let mut c = Conversation::new("d".into(), None, None);
    for (q, a) in [("Q1", "A1"), ("Q2", "A2"), ("Q3", "A3")] {
        c.messages.push(Message::user(q));
        c.messages.push(Message::assistant(a));
    }
    let now = Utc::now();
    c.times = (0..6).map(|i| (i, now)).collect();
    c.ratings = vec![(1, 1), (5, -1)];
    c.bookmarks = vec![(5, "A3".into())];
    // A regenerated third answer: its branch point sits after the deleted exchange.
    keep_branch(&mut c, 4, "answer");
    c.messages.truncate(5);
    c.messages.push(Message::assistant("A3b"));
    // Deleting the first exchange by its answer's index, and asking twice for it, deletes it once.
    assert_eq!(delete_exchanges(&mut c, &[1, 0]).unwrap(), 1);
    let texts: Vec<String> = turns(&c)
        .iter()
        .map(|t| match &t.parts[0] {
            Part::Text { text } => text.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(texts, ["Q2", "A2", "Q3", "A3b"]);
    assert_eq!(c.ratings, vec![(3, -1)]);
    assert_eq!(c.bookmarks[0].0, 3);
    assert_eq!(c.times.len(), 4);
    // The branch point moved with its answer: switching still works.
    let v = view(&c).versions[0].clone();
    assert_eq!(v.from, 2);
    select_version(&mut c, v.from, 0).unwrap();
    assert!(
        matches!(&c.messages[3], Message::Assistant(a) if matches!(a.content.first(), Some(AssistantContent::Text(t)) if t.text == "A3"))
    );
    // Deleting the exchange the branch point is in drops it.
    assert_eq!(delete_exchanges(&mut c, &[3]).unwrap(), 1);
    assert!(c.branches.is_empty());
    assert_eq!(c.messages.len(), 2);
    assert!(delete_exchanges(&mut c, &[9]).is_err());
}
