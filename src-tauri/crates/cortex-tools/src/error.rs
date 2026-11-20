use thiserror::Error;

const TRUNCATED_ERROR_NOTICE: &str =
    "\n[Tool error truncated; middle omitted. Do not repeat a write operation to recover its output.]\n";

fn bound_error_text(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    if max_bytes <= TRUNCATED_ERROR_NOTICE.len() {
        return cortex_store::text::clip(TRUNCATED_ERROR_NOTICE, max_bytes);
    }
    let available = max_bytes - TRUNCATED_ERROR_NOTICE.len();
    let tail_bytes = available / 4;
    let head_end = text.floor_char_boundary(available - tail_bytes);
    let tail_start = text.ceil_char_boundary(text.len() - tail_bytes);
    format!(
        "{}{}{}",
        &text[..head_end],
        TRUNCATED_ERROR_NOTICE,
        &text[tail_start..]
    )
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown connector: {0}")]
    UnknownConnector(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("access denied: {0}")]
    AccessDenied(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("connector not authenticated")]
    NotAuthenticated,
    #[error("connector disabled")]
    ConnectorDisabled,
    #[error("connector is not ready: {0}")]
    NotReady(String),
    #[error("connector is no longer registered")]
    Retired,
    #[error("tool disabled")]
    ToolDisabled,
    #[error("tool call not approved")]
    ApprovalDenied,
    /// The remote could not be reached. Distinct from a remote that answered with an error, which
    /// says nothing about the connection and must not mark the connector as failing.
    #[error("could not reach {0}")]
    Unreachable(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

impl ToolError {
    /// Keep the error kind while limiting the text that can reach the model.
    pub fn bounded_for_model(self, max_bytes: usize) -> Self {
        match self {
            Self::UnknownConnector(text) => Self::UnknownConnector(bound_error_text(
                text,
                max_bytes.saturating_sub("unknown connector: ".len()),
            )),
            Self::UnknownTool(text) => Self::UnknownTool(bound_error_text(
                text,
                max_bytes.saturating_sub("unknown tool: ".len()),
            )),
            Self::AccessDenied(text) => Self::AccessDenied(bound_error_text(
                text,
                max_bytes.saturating_sub("access denied: ".len()),
            )),
            Self::InvalidInput(text) => Self::InvalidInput(bound_error_text(
                text,
                max_bytes.saturating_sub("invalid input: ".len()),
            )),
            Self::NotReady(text) => Self::NotReady(bound_error_text(
                text,
                max_bytes.saturating_sub("connector is not ready: ".len()),
            )),
            Self::Unreachable(text) => Self::Unreachable(bound_error_text(
                text,
                max_bytes.saturating_sub("could not reach ".len()),
            )),
            Self::ExecutionFailed(text) => Self::ExecutionFailed(bound_error_text(
                text,
                max_bytes.saturating_sub("execution failed: ".len()),
            )),
            other => other,
        }
    }

    pub fn to_ipc_string(&self) -> String {
        self.to_string()
    }

    pub fn is_transport(&self) -> bool {
        matches!(self, ToolError::Unreachable(_))
    }

    /// What went wrong, without the text, which can hold URLs, file paths or what a remote said.
    /// For the log file, which users send to support.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownConnector(_) => "unknown connector",
            Self::UnknownTool(_) => "unknown tool",
            Self::AccessDenied(_) => "access denied",
            Self::InvalidInput(_) => "invalid input",
            Self::NotAuthenticated => "not authenticated",
            Self::ConnectorDisabled => "connector disabled",
            Self::NotReady(_) => "not ready",
            Self::Retired => "retired",
            Self::ToolDisabled => "tool disabled",
            Self::ApprovalDenied => "not approved",
            Self::Unreachable(_) => "unreachable",
            Self::ExecutionFailed(_) => "execution failed",
        }
    }
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("unknown connector: {0}")]
    UnknownConnector(String),
    #[error("{0} has no credentials to obtain and no session to open")]
    NothingToConnect(String),
    #[error("{0} has no credentials to forget")]
    NothingToForget(String),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Session(#[from] ToolError),
    #[error("{0}")]
    Failed(String),
}

impl ConnectError {
    pub fn to_ipc_string(&self) -> String {
        self.to_string()
    }

    /// See [`ToolError::kind`].
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownConnector(_) => "unknown connector",
            Self::NothingToConnect(_) => "nothing to connect",
            Self::NothingToForget(_) => "nothing to forget",
            Self::Auth(error) => error.kind(),
            Self::Session(error) => error.kind(),
            Self::Failed(_) => "failed",
        }
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("credentials not configured")]
    RequiresSetup,
    #[error("token expired and refresh failed: {0}")]
    RequiresRefresh(String),
    #[error("authentication cancelled")]
    Cancelled,
    #[error("authentication failed: {0}")]
    Failed(String),
}

impl AuthError {
    pub fn to_ipc_string(&self) -> String {
        self.to_string()
    }

    /// See [`ToolError::kind`].
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RequiresSetup => "requires setup",
            Self::RequiresRefresh(_) => "token refresh failed",
            Self::Cancelled => "cancelled",
            Self::Failed(_) => "authentication failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logged_kind_carries_none_of_the_error_text() {
        let secret = "https://evil.example/?d=salary";
        let errors = [
            ConnectError::Session(ToolError::Unreachable(secret.into())),
            ConnectError::Auth(AuthError::Failed(secret.into())),
            ConnectError::Failed(secret.into()),
        ];
        for error in errors {
            assert!(error.to_string().contains(secret));
            assert!(!error.kind().contains("evil"), "{}", error.kind());
        }
        assert_eq!(
            ConnectError::Session(ToolError::ExecutionFailed(secret.into())).kind(),
            "execution failed"
        );
    }
}
