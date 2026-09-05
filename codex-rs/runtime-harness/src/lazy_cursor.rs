use crate::cursor_acp::CursorAcpBackend;
use crate::cursor_acp::CursorAcpConfig;
use crate::cursor_acp::CursorAcpError;
use crate::router::AgentBackend;
use crate::router::BackendFuture;
use crate::router::RuntimeRouterError;
use crate::types::ProviderId;
use crate::types::RuntimeEventSink;
use crate::types::RuntimeInteractionHandler;
use crate::types::RuntimeModelId;
use crate::types::RuntimeSessionId;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::Semaphore;

/// Lazily owns the Cursor ACP child.
///
/// Constructing the harness never launches Cursor. The first Cursor session
/// operation connects `agent acp`; switching away or rotating credentials calls
/// [`shutdown`](AgentBackend::shutdown), which terminates the child and drops the
/// provider-specific session state.
pub struct LazyCursorBackend {
    config: CursorAcpConfig,
    backend: RwLock<Option<Arc<CursorAcpBackend>>>,
    connect_permit: Arc<Semaphore>,
}

impl LazyCursorBackend {
    pub fn new(config: CursorAcpConfig) -> Self {
        Self {
            config,
            backend: RwLock::new(None),
            connect_permit: Arc::new(Semaphore::new(1)),
        }
    }

    async fn backend(&self) -> Result<Arc<CursorAcpBackend>, RuntimeRouterError> {
        if let Some(backend) = self.backend.read().await.as_ref().cloned() {
            return Ok(backend);
        }

        let _permit = Arc::clone(&self.connect_permit)
            .acquire_owned()
            .await
            .map_err(|_| RuntimeRouterError::ProviderUnavailable(ProviderId::Cursor))?;
        if let Some(backend) = self.backend.read().await.as_ref().cloned() {
            return Ok(backend);
        }

        let backend = Arc::new(CursorAcpBackend::connect(self.config.clone()).await?);
        *self.backend.write().await = Some(Arc::clone(&backend));
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
        self.backend.read().await.is_some()
    }

    fn is_transport_failure(error: &CursorAcpError) -> bool {
        matches!(
            error,
            CursorAcpError::Io(_)
                | CursorAcpError::Json(_)
                | CursorAcpError::UnexpectedEof(_)
                | CursorAcpError::ConnectionUnavailable
        )
    }

    /// Replace one failed ACP child exactly once. Concurrent callers that observe
    /// the same failed Arc converge on the replacement created by the first
    /// caller instead of starting duplicate Cursor processes.
    async fn recover_backend(
        &self,
        failed: &Arc<CursorAcpBackend>,
    ) -> Result<Arc<CursorAcpBackend>, RuntimeRouterError> {
        let _permit = Arc::clone(&self.connect_permit)
            .acquire_owned()
            .await
            .map_err(|_| RuntimeRouterError::ProviderUnavailable(ProviderId::Cursor))?;

        if let Some(current) = self.backend.read().await.as_ref().cloned()
            && !Arc::ptr_eq(&current, failed)
        {
            return Ok(current);
        }

        {
            let mut slot = self.backend.write().await;
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, failed))
            {
                *slot = None;
            }
        }

        // A transport-fatal CursorAcpBackend may already have dropped its
        // connection. Shutdown is best-effort here; the replacement below is the
        // authoritative recovery step.
        let _ = failed.shutdown().await;

        if let Some(current) = self.backend.read().await.as_ref().cloned() {
            return Ok(current);
        }

        let replacement = Arc::new(CursorAcpBackend::connect(self.config.clone()).await?);
        *self.backend.write().await = Some(Arc::clone(&replacement));
        Ok(replacement)
    }

    async fn shutdown_inner(&self) -> Result<(), RuntimeRouterError> {
        let _permit = Arc::clone(&self.connect_permit)
            .acquire_owned()
            .await
            .map_err(|_| RuntimeRouterError::ProviderUnavailable(ProviderId::Cursor))?;
        let backend = self.backend.write().await.take();
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
            match backend.new_session(cwd).await {
                Ok(session) => Ok(session),
                Err(error) if Self::is_transport_failure(&error) => {
                    let recovered = self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/new",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    Ok(recovered.new_session(cwd).await?)
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    fn create_session_for_model<'a>(
        &'a self,
        cwd: &'a Path,
        model: &'a RuntimeModelId,
    ) -> BackendFuture<'a, RuntimeSessionId> {
        Box::pin(async move {
            if model.provider != ProviderId::Cursor {
                return Err(RuntimeRouterError::ModelProviderMismatch {
                    backend: ProviderId::Cursor,
                    model_provider: model.provider,
                });
            }
            let backend = self.backend().await?;
            match backend.new_session_with_model(cwd, &model.model).await {
                Ok(session) => Ok(session),
                Err(error) if Self::is_transport_failure(&error) => {
                    let recovered = self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/new",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    Ok(recovered.new_session_with_model(cwd, &model.model).await?)
                }
                Err(error) => Err(error.into()),
            }
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
            match backend
                .load_session(session_id, cwd, Arc::clone(&sink))
                .await
            {
                Ok(()) => Ok(()),
                Err(error) if Self::is_transport_failure(&error) => {
                    let recovered = self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/load",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    recovered.load_session(session_id, cwd, sink).await?;
                    Ok(())
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    fn load_session_for_model<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        model: &'a RuntimeModelId,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            if model.provider != ProviderId::Cursor {
                return Err(RuntimeRouterError::ModelProviderMismatch {
                    backend: ProviderId::Cursor,
                    model_provider: model.provider,
                });
            }
            let backend = self.backend().await?;
            match backend
                .load_session_with_model(session_id, cwd, &model.model, Arc::clone(&sink))
                .await
            {
                Ok(()) => Ok(()),
                Err(error) if Self::is_transport_failure(&error) => {
                    let recovered = self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/load",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    recovered
                        .load_session_with_model(session_id, cwd, &model.model, sink)
                        .await?;
                    Ok(())
                }
                Err(error) => Err(error.into()),
            }
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
            match backend.prompt(session_id, prompt, sink, interactions).await {
                Ok(stop_reason) => Ok(stop_reason),
                Err(error) if Self::is_transport_failure(&error) => {
                    self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/prompt",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    // A coding turn can already have edited files or executed
                    // tools before its transport failed. Recover the process but
                    // never replay the prompt automatically.
                    Err(RuntimeRouterError::CursorTurnInterrupted(error))
                }
                Err(error) => Err(error.into()),
            }
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
