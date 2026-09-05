use crate::cursor_acp::CursorAcpBackend;
use crate::cursor_acp::CursorAcpConfig;
use crate::router::AgentBackend;
use crate::router::BackendFuture;
use crate::router::RuntimeRouterError;
use crate::types::ProviderId;
use crate::types::RuntimeEventSink;
use crate::types::RuntimeInteractionHandler;
use crate::types::RuntimeSessionId;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Lazily owns the Cursor ACP child.
///
/// Constructing the harness never launches Cursor. The first Cursor session
/// operation connects `agent acp`; switching away or rotating credentials calls
/// [`shutdown`](AgentBackend::shutdown), which terminates the child and drops the
/// provider-specific session state.
pub struct LazyCursorBackend {
    config: CursorAcpConfig,
    backend: Mutex<Option<Arc<CursorAcpBackend>>>,
}

impl LazyCursorBackend {
    pub fn new(config: CursorAcpConfig) -> Self {
        Self {
            config,
            backend: Mutex::new(None),
        }
    }

    async fn backend(&self) -> Result<Arc<CursorAcpBackend>, RuntimeRouterError> {
        let mut slot = self.backend.lock().await;
        if let Some(backend) = slot.as_ref() {
            return Ok(Arc::clone(backend));
        }
        let backend = Arc::new(CursorAcpBackend::connect(self.config.clone()).await?);
        *slot = Some(Arc::clone(&backend));
        Ok(backend)
    }

    /// Drop the current ACP process and force a fresh authenticated child on the
    /// next operation. This is used after account activation and crash recovery.
    pub async fn restart(&self) -> Result<(), RuntimeRouterError> {
        self.shutdown_inner().await?;
        let _ = self.backend().await?;
        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        self.backend.lock().await.is_some()
    }

    async fn shutdown_inner(&self) -> Result<(), RuntimeRouterError> {
        let backend = self.backend.lock().await.take();
        if let Some(backend) = backend {
            backend.shutdown().await?;
        }
        Ok(())
    }
}

impl AgentBackend for LazyCursorBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Cursor
    }

    fn create_session<'a>(&'a self, cwd: &'a Path) -> BackendFuture<'a, RuntimeSessionId> {
        Box::pin(async move {
            let backend = self.backend().await?;
            Ok(backend.new_session(cwd).await?)
        })
    }

    fn load_session<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let backend = self.backend().await?;
            backend.load_session(session_id, cwd, sink).await?;
            Ok(())
        })
    }

    fn prompt<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        prompt: &'a str,
        sink: Arc<dyn RuntimeEventSink>,
        interactions: Arc<dyn RuntimeInteractionHandler>,
    ) -> BackendFuture<'a, Option<String>> {
        Box::pin(async move {
            let backend = self.backend().await?;
            Ok(backend
                .prompt(session_id, prompt, sink, interactions)
                .await?)
        })
    }

    fn cancel<'a>(&'a self, session_id: &'a RuntimeSessionId) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let backend = self.backend().await?;
            backend.cancel(session_id).await?;
            Ok(())
        })
    }

    fn shutdown<'a>(&'a self) -> BackendFuture<'a, ()> {
        Box::pin(async move { self.shutdown_inner().await })
    }
}
