//! The error the chat engine's upper layers return: what storage reports, and what fails beyond
//! it (a download, a model, a request that cannot be done). The commands turn it into the text the
//! window shows, through `From<ChatError> for String`, in one place.

use crate::support::error::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    /// Reading or writing what the app keeps.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A request over the network failed: what was asked for, and why.
    #[error("{what}: {reason}")]
    Network { what: String, reason: String },
    /// A model failed or answered with something unusable.
    #[error("the model: {0}")]
    Model(String),
    /// Something the request asks for that cannot be done, said for the user.
    #[error("{0}")]
    Invalid(String),
    /// A failure with no kind of its own (a library's error, a task that panicked), kept as its
    /// message.
    #[error("{0}")]
    Internal(String),
}

impl ChatError {
    /// The same failure, for another receiver (several jobs waiting on one run): the kind is kept,
    /// except that a storage error, which holds an I/O error that cannot be copied, travels as its
    /// message.
    pub fn copy_for_another(&self) -> Self {
        match self {
            Self::Store(e) => Self::Internal(e.to_string()),
            Self::Network { what, reason } => Self::Network {
                what: what.clone(),
                reason: reason.clone(),
            },
            Self::Model(m) => Self::Model(m.clone()),
            Self::Invalid(m) => Self::Invalid(m.clone()),
            Self::Internal(m) => Self::Internal(m.clone()),
        }
    }

    /// An unclassified failure, as its message: `.map_err(ChatError::internal)`.
    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }

    /// A failed request for `what` (a file, a page, a pack).
    pub fn network(what: impl Into<String>, reason: impl std::fmt::Display) -> Self {
        Self::Network {
            what: what.into(),
            reason: reason.to_string(),
        }
    }
}

impl From<ChatError> for String {
    fn from(error: ChatError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_the_user_sees_them() {
        let missing = ChatError::from(StoreError::NotFound {
            kind: "conversation",
            id: "c1".into(),
        });
        assert_eq!(String::from(missing), "no conversation `c1`");
        assert_eq!(
            String::from(ChatError::network("the emoji pack", "timed out")),
            "the emoji pack: timed out"
        );
        assert_eq!(
            String::from(ChatError::Invalid("Pick a language.".into())),
            "Pick a language."
        );
    }
}
