//! A registered connector behind its handle, and the one operation at a time each connector runs.

use super::*;

/// A registered connector, and whether the registry has since evicted it.
///
/// What the registry stores in place of the connector itself, and where eviction is enforced:
/// `invoke` returns [`ToolError::Retired`] instead of delegating, so no caller can skip the check.
pub struct ConnectorHandle {
    inner: Box<dyn Connector>,
    retired: std::sync::atomic::AtomicBool,
}

impl ConnectorHandle {
    pub(super) fn new(connector: impl Connector + 'static) -> Self {
        Self {
            inner: Box::new(connector),
            retired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub(super) fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl Connector for ConnectorHandle {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn descriptor(&self) -> ConnectorDescriptor {
        self.inner.descriptor()
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        self.inner.tools()
    }

    fn readiness(&self) -> Readiness {
        self.inner.readiness()
    }

    fn vetted(&self) -> bool {
        self.inner.vetted()
    }

    async fn invoke(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        if self.is_retired() {
            return Err(ToolError::Retired);
        }
        self.inner.invoke(tool_name, input).await
    }

    async fn invoke_read_only(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        if self.is_retired() {
            return Err(ToolError::Retired);
        }
        self.inner.invoke_read_only(tool_name, input).await
    }

    fn as_credentialed(&self) -> Option<&dyn Credentialed> {
        self.inner.as_credentialed()
    }

    fn as_session(&self) -> Option<&dyn Session> {
        self.inner.as_session()
    }

    async fn resources(&self) -> Result<Vec<Value>, ToolError> {
        if self.is_retired() {
            return Err(ToolError::Retired);
        }
        self.inner.resources().await
    }

    async fn read_resource(&self, uri: &str) -> Result<String, ToolError> {
        if self.is_retired() {
            return Err(ToolError::Retired);
        }
        self.inner.read_resource(uri).await
    }
}

#[derive(Default)]
pub(super) struct ConnectorOperation {
    pub(super) busy: Arc<tokio::sync::Mutex<()>>,
    pub(super) cancel: tokio::sync::Notify,
    pub(super) cancelled_attempts: RwLock<HashSet<String>>,
}

impl ConnectorOperation {
    pub(super) async fn cancelled(&self, attempt_id: Option<&str>) {
        let Some(attempt_id) = attempt_id else {
            return std::future::pending().await;
        };
        loop {
            let notified = self.cancel.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .cancelled_attempts
                .read()
                .expect("cancellation lock poisoned")
                .contains(attempt_id)
            {
                return;
            }
            notified.await;
        }
    }
}
