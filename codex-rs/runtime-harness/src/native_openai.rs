use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("failed to reload native OpenAI authentication: {message}")]
pub struct NativeOpenAiReloadError {
    message: String,
}

impl NativeOpenAiReloadError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Bridge implemented by the native Codex host so an embedded account switch
/// can invalidate and reload the exact in-memory authentication manager used by
/// subsequent OpenAI turns.
///
/// The runtime harness deliberately does not construct a second AuthManager:
/// reloading a different instance would not update the authenticated clients
/// already owned by the running Codex process.
#[async_trait]
pub trait NativeOpenAiAuthReloader: Send + Sync {
    async fn reload(&self) -> Result<(), NativeOpenAiReloadError>;
}
