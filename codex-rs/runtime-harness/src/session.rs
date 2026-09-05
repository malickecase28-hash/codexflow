use crate::types::PermissionOutcome;
use crate::types::ProviderId;
use crate::types::RuntimeEvent;
use crate::types::RuntimeModelId;
use crate::types::RuntimeSessionId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HarnessEvent {
    UserMessage {
        text: String,
    },
    AssistantMessage {
        text: String,
    },
    ToolRequest {
        provider: ProviderId,
        tool_call_id: Option<String>,
        payload: Value,
    },
    ToolResult {
        provider: ProviderId,
        tool_call_id: Option<String>,
        payload: Value,
    },
    PermissionDecision {
        provider: ProviderId,
        request_id: Value,
        outcome: PermissionOutcome,
    },
    SystemNotice {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationRetryDisposition {
    RetryOriginal,
    ContinueFromRepositoryState,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HarnessSessionError {
    #[error("cannot switch runtime model while a turn is active")]
    TurnActive,
    #[error("model {model} belongs to {actual}, not active provider {expected}")]
    ProviderMismatch {
        model: RuntimeModelId,
        expected: ProviderId,
        actual: ProviderId,
    },
}

/// Provider-independent conversation/session state owned by the Codex harness.
///
/// Backend session identifiers are deliberately disposable. The canonical
/// transcript and working directory survive provider process/account boundaries.
#[derive(Debug, Clone)]
pub struct HarnessSession {
    pub id: String,
    pub transcript: Vec<HarnessEvent>,
    pub working_directory: PathBuf,
    pub provider: ProviderId,
    pub model: RuntimeModelId,
    pub account: Option<String>,
    pub backend_session: Option<RuntimeSessionId>,
    pub backend_generation: u64,
    turn_active: bool,
    turn_side_effect_observed: bool,
}

impl HarnessSession {
    pub fn new(id: impl Into<String>, working_directory: PathBuf, model: RuntimeModelId) -> Self {
        Self {
            id: id.into(),
            transcript: Vec::new(),
            working_directory,
            provider: model.provider,
            model,
            account: None,
            backend_session: None,
            backend_generation: 0,
            turn_active: false,
            turn_side_effect_observed: false,
        }
    }

    pub fn begin_turn(&mut self, user_message: impl Into<String>) {
        self.turn_active = true;
        self.turn_side_effect_observed = false;
        self.transcript.push(HarnessEvent::UserMessage {
            text: user_message.into(),
        });
    }

    pub fn finish_turn(&mut self, assistant_message: Option<String>) {
        if let Some(text) = assistant_message {
            self.transcript
                .push(HarnessEvent::AssistantMessage { text });
        }
        self.turn_active = false;
        self.turn_side_effect_observed = false;
    }

    /// Conservatively mark a turn unsafe to replay when provider/tool activity
    /// may already have changed external state. The caller may also call
    /// `mark_side_effect` for native Codex events that carry stronger semantics.
    pub fn observe_runtime_event(&mut self, event: &RuntimeEvent) {
        if matches!(
            event,
            RuntimeEvent::ToolCall { .. } | RuntimeEvent::ToolCallUpdate { .. }
        ) {
            self.turn_side_effect_observed = true;
        }
    }

    pub fn mark_side_effect(&mut self) {
        self.turn_side_effect_observed = true;
    }

    pub fn rotation_retry_disposition(&self) -> RotationRetryDisposition {
        if self.turn_side_effect_observed {
            RotationRetryDisposition::ContinueFromRepositoryState
        } else {
            RotationRetryDisposition::RetryOriginal
        }
    }

    pub fn record_event(&mut self, event: HarnessEvent) {
        self.transcript.push(event);
    }

    pub fn switch_model(&mut self, model: RuntimeModelId) -> Result<(), HarnessSessionError> {
        if self.turn_active {
            return Err(HarnessSessionError::TurnActive);
        }
        let provider_changed = model.provider != self.provider;
        self.provider = model.provider;
        self.model = model;
        if provider_changed {
            self.account = None;
            self.invalidate_backend();
        }
        Ok(())
    }

    pub fn select_model_within_provider(
        &mut self,
        model: RuntimeModelId,
    ) -> Result<(), HarnessSessionError> {
        if model.provider != self.provider {
            return Err(HarnessSessionError::ProviderMismatch {
                actual: model.provider,
                expected: self.provider,
                model,
            });
        }
        self.switch_model(model)
    }

    pub fn bind_backend_session(&mut self, session: RuntimeSessionId, generation: u64) {
        self.backend_session = Some(session);
        self.backend_generation = generation;
    }

    /// Account changes define a runtime authentication boundary. The canonical
    /// transcript survives, while provider-specific backend session state is
    /// invalidated and must be recreated/reloaded before the next prompt.
    pub fn account_changed(&mut self, account: impl Into<String>, generation: u64) {
        self.account = Some(account.into());
        self.backend_generation = generation;
        self.backend_session = None;
    }

    pub fn backend_generation_matches(&self, generation: u64) -> bool {
        self.backend_generation == generation
    }

    pub fn invalidate_backend(&mut self) {
        self.backend_session = None;
        self.backend_generation = 0;
    }

    pub fn is_turn_active(&self) -> bool {
        self.turn_active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: ProviderId, name: &str) -> RuntimeModelId {
        RuntimeModelId::new(provider, name).unwrap()
    }

    #[test]
    fn provider_switch_preserves_transcript_but_invalidates_backend_identity() {
        let mut session = HarnessSession::new(
            "session",
            PathBuf::from("/tmp/project"),
            model(ProviderId::OpenAi, "gpt-5"),
        );
        session.record_event(HarnessEvent::UserMessage {
            text: "hello".to_string(),
        });
        session.account = Some("openai-account".to_string());
        session.bind_backend_session(RuntimeSessionId("native".to_string()), 4);

        session
            .switch_model(model(ProviderId::Cursor, "auto"))
            .unwrap();

        assert_eq!(session.transcript.len(), 1);
        assert_eq!(session.provider, ProviderId::Cursor);
        assert!(session.account.is_none());
        assert!(session.backend_session.is_none());
        assert_eq!(session.backend_generation, 0);
    }

    #[test]
    fn account_rotation_keeps_canonical_transcript_and_drops_backend_session() {
        let mut session = HarnessSession::new(
            "session",
            PathBuf::from("/tmp/project"),
            model(ProviderId::Cursor, "auto"),
        );
        session.record_event(HarnessEvent::AssistantMessage {
            text: "work completed".to_string(),
        });
        session.bind_backend_session(RuntimeSessionId("cursor-old".to_string()), 1);
        session.account_changed("cursor-account-b", 2);

        assert_eq!(session.transcript.len(), 1);
        assert_eq!(session.account.as_deref(), Some("cursor-account-b"));
        assert!(session.backend_session.is_none());
        assert!(session.backend_generation_matches(2));
    }

    #[test]
    fn side_effectful_turn_is_never_blindly_replayed_after_rotation() {
        let mut session = HarnessSession::new(
            "session",
            PathBuf::from("/tmp/project"),
            model(ProviderId::Cursor, "auto"),
        );
        session.begin_turn("edit the file");
        session.observe_runtime_event(&RuntimeEvent::ToolCall {
            raw: serde_json::json!({"toolCallId":"edit-1"}),
        });
        assert_eq!(
            session.rotation_retry_disposition(),
            RotationRetryDisposition::ContinueFromRepositoryState
        );
    }

    #[test]
    fn model_switch_waits_until_turn_is_idle() {
        let mut session = HarnessSession::new(
            "session",
            PathBuf::from("/tmp/project"),
            model(ProviderId::OpenAi, "gpt-5"),
        );
        session.begin_turn("hello");
        assert!(matches!(
            session.switch_model(model(ProviderId::Cursor, "auto")),
            Err(HarnessSessionError::TurnActive)
        ));
        assert_eq!(session.provider, ProviderId::OpenAi);
    }
}
