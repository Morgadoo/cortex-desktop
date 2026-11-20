//! The tools the assistant works with: the code sandbox and workbench, written and custom tools, OpenAPI servers, computer use, web pages, the terminal, coding sessions, canvases, spreadsheets and proposals.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod canvas;
pub mod code;
pub mod computer;
pub mod custom_tools;
pub mod openapi;
pub mod outputs;
pub mod proposals;
pub mod sandbox;
pub mod script_tools;
pub mod sheets;
pub mod terminal;
pub mod webpage;
pub mod workbench;
