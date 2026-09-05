use crate::types::ProviderId;
use serde::Deserialize;
use serde::Serialize;

/// Provider execution capabilities advertised to the harness/TUI.
///
/// These fields intentionally match the V1 architecture descriptor so feature
/// availability is data-driven rather than inferred from provider names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tools: bool,
    pub permissions: bool,
    pub resume: bool,
    pub models: bool,
    pub model_parameters: bool,
    pub cancel: bool,
    pub account_hot_reload: bool,
}

impl ProviderCapabilities {
    pub const fn for_provider(provider: ProviderId) -> Self {
        match provider {
            ProviderId::OpenAi => Self {
                streaming: true,
                tools: true,
                permissions: true,
                resume: true,
                models: true,
                model_parameters: true,
                cancel: true,
                account_hot_reload: true,
            },
            ProviderId::Cursor => Self {
                streaming: true,
                tools: true,
                permissions: true,
                resume: true,
                models: true,
                model_parameters: true,
                cancel: true,
                // Cursor ACP is conservatively restarted after account changes;
                // the child may have cached credentials.
                account_hot_reload: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_supports_in_process_account_reload() {
        let capabilities = ProviderCapabilities::for_provider(ProviderId::OpenAi);
        assert!(capabilities.streaming);
        assert!(capabilities.tools);
        assert!(capabilities.permissions);
        assert!(capabilities.account_hot_reload);
    }

    #[test]
    fn cursor_requires_runtime_restart_after_account_change() {
        let capabilities = ProviderCapabilities::for_provider(ProviderId::Cursor);
        assert!(capabilities.streaming);
        assert!(capabilities.permissions);
        assert!(capabilities.cancel);
        assert!(!capabilities.account_hot_reload);
    }
}
