//! Images, audio, video and animation: generation, speech, recording, and the dictation app connection.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod animation;
pub mod audio;
pub mod hex;
pub mod media;
pub mod recorder;
pub mod video;
pub mod videogen;
