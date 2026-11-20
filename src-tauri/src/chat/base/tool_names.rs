//! The names of the tools the app provides itself, in the bottom layer: the built-ins answer to
//! them, and written tools may not take them.

pub const WEB_SEARCH: &str = "web_search";
pub const REMEMBER: &str = "remember";
pub const FORGET: &str = "forget";
pub const USE_SKILL: &str = "use_skill";
pub const UPDATE_PLAN: &str = "update_plan";
/// Run by the engine, which holds the model the workers need.
pub const DELEGATE: &str = "delegate";
/// Run by the engine, which knows the conversation the canvas belongs to.
pub const CANVAS: &str = "canvas";
pub const SCHEDULE_TASK: &str = "schedule_task";
pub const SEARCH_LIBRARY: &str = "search_library";
pub const READ_DOCUMENT: &str = "read_document";
pub const SAVE_SKILL: &str = "save_skill";
pub const KNOWLEDGE_WRITE: &str = "knowledge_write";
pub const KNOWLEDGE_READ: &str = "knowledge_read";
pub const KNOWLEDGE_SEARCH: &str = "knowledge_search";
pub const SEARCH_CHATS: &str = "search_chats";
pub const READ_CHAT: &str = "read_chat";
pub const NOTIFY: &str = "notify";
pub const CALENDAR_EVENTS: &str = "calendar_events";
pub const FIND_CONTACT: &str = "find_contact";
pub const MEMORY_SEARCH: &str = "memory_search";
/// The assistant's recent runs, filtered.
pub const QUERY_TRACES: &str = "query_traces";
/// One run in full: its model and tool calls, and what it had in front of it and used.
pub const GET_RUN: &str = "get_run";
/// The assistant's own run history: what it used and what it cost.
pub const USAGE_STATS: &str = "usage_stats";
/// What failed in the assistant's recent runs, most often first.
pub const FAILURE_SUMMARY: &str = "failure_summary";

/// The built-ins `builtins::run` answers.
pub const NAMES: &[&str] = &[
    QUERY_TRACES,
    GET_RUN,
    MEMORY_SEARCH,
    USAGE_STATS,
    FAILURE_SUMMARY,
    WEB_SEARCH,
    CALENDAR_EVENTS,
    FIND_CONTACT,
    REMEMBER,
    FORGET,
    USE_SKILL,
    UPDATE_PLAN,
    SCHEDULE_TASK,
    SEARCH_LIBRARY,
    READ_DOCUMENT,
    SAVE_SKILL,
    KNOWLEDGE_WRITE,
    KNOWLEDGE_READ,
    KNOWLEDGE_SEARCH,
    SEARCH_CHATS,
    READ_CHAT,
];
