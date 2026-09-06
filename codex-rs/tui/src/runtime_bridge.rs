use crate::legacy_core::config::Config;
use codex_runtime_harness::CursorAcpConfig;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::RuntimeHarness;
use codex_runtime_harness::RuntimeModelId;
use color_eyre::eyre::Result;
use std::sync::Arc;

const RUNTIME_SELECTION_FILE: &str = "runtime-harness-selection.json";

/// Process-lifetime bridge between Codex's TUI and the provider-neutral runtime harness.
///
/// The bridge is constructed once during TUI startup and intentionally keeps native
/// OpenAI execution outside the harness. Cursor remains lazy: constructing this type
/// does not launch `agent acp`; the child starts only when a Cursor route is used.
pub(crate) struct RuntimeBridge {
    harness: Arc<RuntimeHarness>,
}

impl RuntimeBridge {
    pub(crate) fn new(config: &Config, default_openai_model: &str) -> Result<Self> {
        let default_model = RuntimeModelId::new(ProviderId::OpenAi, default_openai_model)?;
        let selection_path = config
            .codex_home
            .to_path_buf()
            .join(RUNTIME_SELECTION_FILE);
        let cursor_config = CursorAcpConfig {
            process_cwd: Some(config.cwd.to_path_buf()),
            ..Default::default()
        };
        let harness = RuntimeHarness::embedded(default_model, selection_path, cursor_config)?;
        Ok(Self {
            harness: Arc::new(harness),
        })
    }

    /// Deterministically terminate any provider-owned child before app-server exits.
    pub(crate) async fn shutdown(&self) -> Result<()> {
        self.harness
            .router()
            .shutdown_provider(ProviderId::Cursor)
            .await?;
        Ok(())
    }
}
