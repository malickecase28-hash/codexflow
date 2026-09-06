use crate::legacy_core::config::Config;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ReloadAccountAuthResponse;
use codex_app_server_protocol::RequestId;
use codex_runtime_harness::CursorAcpConfig;
use codex_runtime_harness::NativeOpenAiAuthReloader;
use codex_runtime_harness::NativeOpenAiReloadError;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::RuntimeHarness;
use codex_runtime_harness::RuntimeModelId;
use color_eyre::eyre::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

const RUNTIME_SELECTION_FILE: &str = "runtime-harness-selection.json";

/// Reloads the exact app-server-owned authentication manager used by native
/// OpenAI turns. The request contains no credential material; it only asks the
/// running app-server to re-read the native credential store after subswap has
/// activated a different account.
struct AppServerOpenAiAuthReloader {
    request_handle: AppServerRequestHandle,
}

impl AppServerOpenAiAuthReloader {
    fn new(request_handle: AppServerRequestHandle) -> Self {
        Self { request_handle }
    }
}

impl NativeOpenAiAuthReloader for AppServerOpenAiAuthReloader {
    fn reload(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<(), NativeOpenAiReloadError>> + Send + '_,
        >,
    > {
        Box::pin(async move {
            let request_id = RequestId::String(format!(
                "runtime-auth-reload-{}",
                Uuid::new_v4()
            ));
            self.request_handle
                .request_typed::<ReloadAccountAuthResponse>(ClientRequest::ReloadAccountAuth {
                    request_id,
                    params: None,
                })
                .await
                .map(|_| ())
                .map_err(|error| NativeOpenAiReloadError::new(error.to_string()))
        })
    }
}

/// Process-lifetime bridge between Codex's TUI and the provider-neutral runtime harness.
///
/// The bridge is constructed once during TUI startup and intentionally keeps native
/// OpenAI execution outside the harness. Cursor remains lazy: constructing this type
/// does not launch `agent acp`; the child starts only when a Cursor route is used.
pub(crate) struct RuntimeBridge {
    harness: Arc<RuntimeHarness>,
}

impl RuntimeBridge {
    pub(crate) fn new(
        config: &Config,
        default_openai_model: &str,
        app_server_request_handle: AppServerRequestHandle,
    ) -> Result<Self> {
        let default_model = RuntimeModelId::new(ProviderId::OpenAi, default_openai_model)?;
        let selection_path = config
            .codex_home
            .to_path_buf()
            .join(RUNTIME_SELECTION_FILE);
        let cursor_config = CursorAcpConfig {
            process_cwd: Some(config.cwd.to_path_buf()),
            ..Default::default()
        };
        let openai_auth_reloader = Arc::new(AppServerOpenAiAuthReloader::new(
            app_server_request_handle,
        ));
        let harness = RuntimeHarness::embedded_with_openai_reloader(
            default_model,
            selection_path,
            cursor_config,
            openai_auth_reloader,
        )?;
        Ok(Self {
            harness: Arc::new(harness),
        })
    }

    /// Deterministically terminate any provider-owned child before app-server exits.
    pub(crate) async fn shutdown(&self) -> Result<()> {
        self.harness.shutdown().await?;
        Ok(())
    }
}
