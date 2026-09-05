use crate::cursor_acp::CursorAcpBackend;
use crate::cursor_acp::CursorAcpError;
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
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeRouterError {
    #[error("Cursor ACP backend error: {0}")]
    Cursor(#[from] CursorAcpError),
    #[error("runtime provider {0} is unavailable")]
    ProviderUnavailable(ProviderId),
}

/// Route selected models by their explicit provider identity.
///
/// OpenAI remains a native Codex route: callers continue through the existing
/// Codex execution pipeline. External providers are returned as AgentBackend
/// implementations. There is intentionally no silent cross-provider fallback.
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
}
