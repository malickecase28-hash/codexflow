use crate::types::ProviderId;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub native_codex_execution: bool,
    pub resumable_sessions: bool,
    pub streaming_updates: bool,
    pub tool_calls: bool,
    pub permission_requests: bool,
    pub provider_extensions: bool,
    pub account_switching: bool,
    pub quota_reporting: bool,
}

impl ProviderCapabilities {
    pub const fn for_provider(provider: ProviderId) -> Self {
        match provider {
            ProviderId::OpenAi => Self {
                native_codex_execution: true,
                resumable_sessions: true,
                streaming_updates: true,
                tool_calls: true,
                permission_requests: true,
                provider_extensions: false,
                account_switching: true,
                quota_reporting: true,
            },
            ProviderId::Cursor => Self {
                native_codex_execution: false,
                resumable_sessions: true,
                streaming_updates: true,
                tool_calls: true,
                permission_requests: true,
                provider_extensions: true,
                account_switching: true,
                quota_reporting: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_is_marked_as_native() {
        let capabilities = ProviderCapabilities::for_provider(ProviderId::OpenAi);
        assert!(capabilities.native_codex_execution);
        assert!(!capabilities.provider_extensions);
    }

    #[test]
    fn cursor_is_external_and_permission_capable() {
        let capabilities = ProviderCapabilities::for_provider(ProviderId::Cursor);
        assert!(!capabilities.native_codex_execution);
        assert!(capabilities.permission_requests);
        assert!(capabilities.provider_extensions);
    }
}
