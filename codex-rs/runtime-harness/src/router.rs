use crate::cursor_acp::CursorAcpBackend;
use crate::cursor_acp::CursorAcpConfig;
use crate::cursor_acp::CursorAcpError;
use crate::lazy_cursor::LazyCursorBackend;
use crate::types::ProviderId;
use crate::types::RuntimeEventSink;
use crate::types::RuntimeInteractionHandler;
use crate::types::RuntimeModelId;
use crate::types::RuntimeSessionId;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

pub type BackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RuntimeRouterError>> + Send + 'a>>;

/// Provider-neutral execution contract for child/external agent runtimes.
///
/// Native OpenAI/Codex execution intentionally remains in the existing Codex
/// thread pipeline; [`RuntimeRoute::NativeOpenAi`] is the explicit bridge back to
/// that path. External runtimes implement this contract.
pub trait AgentBackend: Send + Sync {
    fn provider(&self) -> ProviderId;

    fn create_session<'a>(&'a self, cwd: &'a Path) -> BackendFuture<'a, RuntimeSessionId>;

    fn load_session<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()>;

    fn prompt<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        prompt: &'a str,
        sink: Arc<dyn RuntimeEventSink>,
        interactions: Arc<dyn RuntimeInteractionHandler>,
    ) -> BackendFuture<'a, Option<String>>;

    fn cancel<'a>(&'a self, session_id: &'a RuntimeSessionId) -> BackendFuture<'a, ()>;

    fn shutdown<'a>(&'a self) -> BackendFuture<'a, ()>;
}

impl AgentBackend for CursorAcpBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Cursor
    }

    fn create_session<'a>(&'a self, cwd: &'a Path) -> BackendFuture<'a, RuntimeSessionId> {
        Box::pin(async move { Ok(self.new_session(cwd).await?) })
    }

    fn load_session<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.load_session(session_id, cwd, sink).await?;
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
        Box::pin(async move { Ok(self.prompt(session_id, prompt, sink, interactions).await?) })
    }

    fn cancel<'a>(&'a self, session_id: &'a RuntimeSessionId) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.cancel(session_id).await?;
            Ok(())
        })
    }

    fn shutdown<'a>(&'a self) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.shutdown().await?;
            Ok(())
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeRouterError {
    #[error("Cursor ACP backend error: {0}")]
    Cursor(#[from] CursorAcpError),
    #[error("runtime provider {0} is unavailable")]
    ProviderUnavailable(ProviderId),
}

/// Route selected models by explicit provider identity.
///
/// The router does not infer a provider from a model name and never performs
/// cross-provider fallback. Cursor can be installed as a lazy backend so startup
/// does not launch `agent acp` until a Cursor operation is actually requested.
pub struct RuntimeRouter {
    cursor: Option<Arc<dyn AgentBackend>>,
}

pub enum RuntimeRoute {
    NativeOpenAi,
    External(Arc<dyn AgentBackend>),
}

impl RuntimeRouter {
    pub fn native_only() -> Self {
        Self { cursor: None }
    }

    pub fn with_cursor(cursor: Arc<CursorAcpBackend>) -> Self {
        Self {
            cursor: Some(cursor),
        }
    }

    pub fn with_lazy_cursor(config: CursorAcpConfig) -> Self {
        Self {
            cursor: Some(Arc::new(LazyCursorBackend::new(config))),
        }
    }

    pub fn route(&self, model: &RuntimeModelId) -> Result<RuntimeRoute, RuntimeRouterError> {
        match model.provider {
            ProviderId::OpenAi => Ok(RuntimeRoute::NativeOpenAi),
            ProviderId::Cursor => self
                .cursor
                .as_ref()
                .cloned()
                .map(RuntimeRoute::External)
                .ok_or(RuntimeRouterError::ProviderUnavailable(ProviderId::Cursor)),
        }
    }

    /// Terminate any provider-owned child process. Native OpenAI has no child
    /// owned by this router, so shutdown is a no-op for that provider.
    pub async fn shutdown_provider(&self, provider: ProviderId) -> Result<(), RuntimeRouterError> {
        match provider {
            ProviderId::OpenAi => Ok(()),
            ProviderId::Cursor => match self.cursor.as_ref() {
                Some(cursor) => cursor.shutdown().await,
                None => Ok(()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_is_always_native() {
        let router = RuntimeRouter::native_only();
        let model = RuntimeModelId::new(ProviderId::OpenAi, "gpt-5").unwrap();
        assert!(matches!(
            router.route(&model).unwrap(),
            RuntimeRoute::NativeOpenAi
        ));
    }

    #[test]
    fn cursor_never_silently_falls_back_to_openai() {
        let router = RuntimeRouter::native_only();
        let model = RuntimeModelId::new(ProviderId::Cursor, "auto").unwrap();
        let error = match router.route(&model) {
            Ok(_) => panic!("cursor must not silently route to OpenAI"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            RuntimeRouterError::ProviderUnavailable(ProviderId::Cursor)
        ));
    }

    #[tokio::test]
    async fn lazy_cursor_router_does_not_spawn_during_construction_or_shutdown() {
        let router = RuntimeRouter::with_lazy_cursor(CursorAcpConfig {
            executable: Some("/definitely/missing/cursor-agent".into()),
            process_cwd: None,
        });
        let model = RuntimeModelId::new(ProviderId::Cursor, "auto").unwrap();
        assert!(matches!(router.route(&model), Ok(RuntimeRoute::External(_))));
        // No operation has touched the backend yet, so shutdown must remain a
        // successful no-op even though the configured executable does not exist.
        router.shutdown_provider(ProviderId::Cursor).await.unwrap();
    }
}
