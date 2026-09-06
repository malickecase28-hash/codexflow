use crate::legacy_core::config::Config;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ReloadAccountAuthResponse;
use codex_app_server_protocol::RequestId;
use codex_runtime_harness::CursorAcpConfig;
use codex_runtime_harness::ModelDescriptor;
use codex_runtime_harness::NativeOpenAiAuthReloader;
use codex_runtime_harness::NativeOpenAiReloadError;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::ProviderQuotaSnapshot;
use codex_runtime_harness::QuotaUpdateSink;
use codex_runtime_harness::RuntimeAutoSwapDecision;
use codex_runtime_harness::RuntimeHarness;
use codex_runtime_harness::RuntimeModelId;
use codex_runtime_harness::RuntimeSelection;
use color_eyre::eyre::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

const RUNTIME_SELECTION_FILE: &str = "runtime-harness-selection.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAccountSummary {
    pub(crate) provider: ProviderId,
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) active: bool,
}

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
    quota_shutdown: watch::Sender<bool>,
    quota_tasks: Vec<JoinHandle<()>>,
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
        let harness = Arc::new(RuntimeHarness::embedded_with_openai_reloader(
            default_model,
            selection_path,
            cursor_config,
            openai_auth_reloader,
        )?);

        let poll_interval = harness.quota_poll_interval();
        let (quota_shutdown, _) = watch::channel(false);
        let mut quota_tasks = Vec::with_capacity(2);
        for provider in [ProviderId::OpenAi, ProviderId::Cursor] {
            let quota_service = Arc::clone(harness.quota_service());
            let shutdown = quota_shutdown.subscribe();
            let sink: Arc<dyn QuotaUpdateSink> =
                Arc::new(|_snapshot: ProviderQuotaSnapshot| {});
            quota_tasks.push(tokio::spawn(async move {
                if let Err(error) = quota_service
                    .run_provider(provider, poll_interval, shutdown, sink)
                    .await
                {
                    tracing::warn!(
                        %provider,
                        error = %error,
                        "runtime quota poller stopped"
                    );
                }
            }));
        }

        Ok(Self {
            harness,
            quota_shutdown,
            quota_tasks,
        })
    }

    pub(crate) async fn selection(&self) -> RuntimeSelection {
        self.harness.selection().await
    }

    pub(crate) async fn refresh_cursor_models(&self) -> Result<Vec<ModelDescriptor>> {
        Ok(self.harness.refresh_cursor_models().await?)
    }

    pub(crate) async fn select_model(&self, model: RuntimeModelId) -> Result<RuntimeSelection> {
        Ok(self.harness.select_model(model).await?)
    }

    pub(crate) async fn select_provider(
        &self,
        model: RuntimeModelId,
    ) -> Result<RuntimeSelection> {
        Ok(self.harness.select_provider(model).await?)
    }

    pub(crate) async fn list_accounts(
        &self,
        provider: ProviderId,
    ) -> Result<Vec<RuntimeAccountSummary>> {
        Ok(self
            .harness
            .broker()
            .list_accounts(provider)
            .await?
            .into_iter()
            .map(|account| RuntimeAccountSummary {
                provider,
                id: account.id.0,
                label: account.label,
                active: account.active,
            })
            .collect())
    }

    pub(crate) async fn use_account(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<RuntimeSelection> {
        self.harness
            .activate_account(provider, account_id.into())
            .await?;
        Ok(self.harness.selection().await)
    }

    pub(crate) async fn remove_account(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<u64> {
        Ok(self
            .harness
            .remove_account(provider, account_id.into())
            .await?)
    }

    pub(crate) async fn quota_snapshot(
        &self,
        provider: ProviderId,
    ) -> Result<ProviderQuotaSnapshot> {
        Ok(self.harness.quota_snapshot(provider).await?)
    }

    pub(crate) async fn auto_swap_default(&self) -> Result<RuntimeAutoSwapDecision> {
        Ok(self.harness.auto_swap_current_default().await?)
    }

    pub(crate) async fn login_cursor(&self, label_hint: Option<String>) -> Result<RuntimeSelection> {
        self.harness.login_cursor(label_hint).await?;
        Ok(self.harness.selection().await)
    }

    /// Stop quota pollers before terminating any provider-owned child process.
    pub(crate) async fn shutdown(&mut self) -> Result<()> {
        let _ = self.quota_shutdown.send(true);
        while let Some(task) = self.quota_tasks.pop() {
            if let Err(error) = task.await {
                tracing::warn!(error = %error, "runtime quota poller join failed");
            }
        }
        self.harness.shutdown().await?;
        Ok(())
    }
}
