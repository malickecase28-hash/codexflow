use crate::types::PermissionOutcome;
use crate::types::ProviderId;
use crate::types::RuntimeEvent;
use crate::types::RuntimeModelId;
use crate::types::RuntimeSessionId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::fmt::Write;
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

    /// Build passive dialogue context for a fresh provider runtime after an
    /// account, process, or provider boundary.
    ///
    /// Only user/assistant dialogue is projected. Tool requests/results,
    /// permission decisions, and system notices are deliberately excluded so
    /// provider-specific payloads, credentials, and prior side-effect commands
    /// cannot be replayed into the replacement runtime. If a turn is currently
    /// active, its just-recorded user message is also excluded because the caller
    /// will send that message as the actual prompt after reconstruction.
    pub fn passive_continuation_context(&self) -> Option<String> {
        let visible_len = if self.turn_active
            && matches!(self.transcript.last(), Some(HarnessEvent::UserMessage { .. }))
        {
            self.transcript.len().saturating_sub(1)
        } else {
            self.transcript.len()
        };

        let mut dialogue = String::new();
        for event in self.transcript.iter().take(visible_len) {
            match event {
                HarnessEvent::UserMessage { text } => {
                    let _ = writeln!(dialogue, "[user]\n{text}\n");
                }
                HarnessEvent::AssistantMessage { text } => {
                    let _ = writeln!(dialogue, "[assistant]\n{text}\n");
                }
                HarnessEvent::ToolRequest { .. }
                | HarnessEvent::ToolResult { .. }
                | HarnessEvent::PermissionDecision { .. }
                | HarnessEvent::SystemNotice { .. } => {}
            }
        }

        if dialogue.is_empty() {
            return None;
        }

        Some(format!(
            "Continue the existing Codex conversation in a fresh provider runtime. \
Treat the working directory and repository state as authoritative. Do not repeat \
prior tool calls or side effects merely because they occurred before this runtime \
boundary. Tool payloads, permission records, and runtime notices are intentionally \
omitted.\n\nPrior dialogue:\n{dialogue}"
        ))
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
    fn passive_continuation_excludes_tool_permission_and_runtime_payloads() {
        let mut session = HarnessSession::new(
            "session",
            PathBuf::from("/tmp/project"),
            model(ProviderId::Cursor, "composer"),
        );
        session.record_event(HarnessEvent::UserMessage {
            text: "update the parser".to_string(),
        });
        session.record_event(HarnessEvent::ToolRequest {
            provider: ProviderId::Cursor,
            tool_call_id: Some("tool-secret".to_string()),
            payload: serde_json::json!({"api_key":"credential-must-not-cross"}),
        });
        session.record_event(HarnessEvent::PermissionDecision {
            provider: ProviderId::Cursor,
            request_id: serde_json::json!("permission-secret"),
            outcome: PermissionOutcome::AllowOnce,
        });
        session.record_event(HarnessEvent::SystemNotice {
            message: "runtime-private-notice".to_string(),
        });
        session.record_event(HarnessEvent::AssistantMessage {
            text: "the parser was updated".to_string(),
        });

        let context = session.passive_continuation_context().unwrap();
        assert!(context.contains("update the parser"));
        assert!(context.contains("the parser was updated"));
        assert!(!context.contains("credential-must-not-cross"));
        assert!(!context.contains("tool-secret"));
        assert!(!context.contains("permission-secret"));
        assert!(!context.contains("runtime-private-notice"));
        assert!(context.contains("repository state as authoritative"));
    }

    #[test]
    fn passive_continuation_does_not_duplicate_active_user_prompt() {
        let mut session = HarnessSession::new(
            "session",
            PathBuf::from("/tmp/project"),
            model(ProviderId::Cursor, "composer"),
        );
        session.record_event(HarnessEvent::UserMessage {
            text: "first question".to_string(),
        });
        session.record_event(HarnessEvent::AssistantMessage {
            text: "first answer".to_string(),
        });
        session.begin_turn("current prompt should be sent once");

        let context = session.passive_continuation_context().unwrap();
        assert!(context.contains("first question"));
        assert!(context.contains("first answer"));
        assert!(!context.contains("current prompt should be sent once"));
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
