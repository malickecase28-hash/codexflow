use crate::RuntimeHarness;
use crate::RuntimeHarnessError;
use crate::types::ProviderId;
use subswap_core::PolicyConfig;
use subswap_core::PolicyDecision;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeAutoSwapStatus {
    pub enabled: bool,
    pub threshold: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAutoSwapDecision {
    Stay {
        provider: ProviderId,
        reason: String,
    },
    Swap {
        provider: ProviderId,
        from: Option<String>,
        to: String,
        reason: String,
    },
    Degraded {
        provider: ProviderId,
        reason: String,
    },
}

impl RuntimeHarness {
    pub fn auto_swap_status(&self) -> RuntimeAutoSwapStatus {
        let settings = subswap_core::settings::current();
        RuntimeAutoSwapStatus {
            enabled: settings.auto_swap.enabled,
            threshold: settings.auto_swap.threshold,
        }
    }

    pub fn set_auto_swap_enabled(
        &self,
        enabled: bool,
    ) -> Result<RuntimeAutoSwapStatus, RuntimeHarnessError> {
        subswap_core::settings::set_auto_swap_enabled(enabled)
            .map_err(crate::AccountBrokerError::from)?;
        subswap_core::settings::reload_from_file().map_err(crate::AccountBrokerError::from)?;
        Ok(self.auto_swap_status())
    }

    /// Evaluate the pinned subswap default policy for the active provider only.
    ///
    /// The underlying harness activation path is used when a swap is selected,
    /// so Cursor child invalidation and native OpenAI auth reload remain part of
    /// the same transactional transition. Cross-provider fallback is never
    /// considered by this entry point.
    pub async fn auto_swap_current_default(
        &self,
    ) -> Result<RuntimeAutoSwapDecision, RuntimeHarnessError> {
        let provider = self.selection().await.provider();
        let decision = self.auto_swap_current(&PolicyConfig::default()).await?;
        Ok(match decision {
            PolicyDecision::NoOp { reason } => RuntimeAutoSwapDecision::Stay {
                provider,
                reason,
            },
            PolicyDecision::Swap { from, to, reason } => RuntimeAutoSwapDecision::Swap {
                provider,
                from: from.map(|id| id.0),
                to: to.0,
                reason,
            },
            PolicyDecision::Degraded { reason } => RuntimeAutoSwapDecision::Degraded {
                provider,
                reason,
            },
        })
    }
}
