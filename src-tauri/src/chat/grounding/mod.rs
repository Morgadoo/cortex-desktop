//! What answers are grounded in: memories, knowledge pages, libraries, embeddings and extraction, retrieval, web and chat search, reading pages, link previews, favicons, learning and calendars.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod calendar;
pub mod embeddings;
pub mod extraction;
pub(super) mod favicon;
pub mod knowledge;
pub mod learning;
pub mod libraries;
pub mod memory;
pub(super) mod preview;
pub mod reader;
pub mod retrieval;
pub mod search;
pub mod search_all;
