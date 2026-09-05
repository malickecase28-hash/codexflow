use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    OpenAi,
    Cursor,
}

impl ProviderId {
    pub const fn subswap_id(self) -> &'static str {
        match self {
            Self::OpenAi => "codex",
            Self::Cursor => "cursor",
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OpenAi => "openai",
            Self::Cursor => "cursor",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown runtime provider '{0}'")]
pub struct ProviderParseError(pub String);

impl FromStr for ProviderId {
    type Err = ProviderParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" | "codex" => Ok(Self::OpenAi),
            "cursor" => Ok(Self::Cursor),
            _ => Err(ProviderParseError(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeModelId {
    pub provider: ProviderId,
    pub model: String,
}

impl RuntimeModelId {
    pub fn new(provider: ProviderId, model: impl Into<String>) -> Result<Self, ModelIdError> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(ModelIdError::EmptyModel);
        }
        Ok(Self { provider, model })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelIdError {
    #[error("runtime model id cannot be empty")]
    EmptyModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeSessionId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionOutcome {
    AllowOnce,
    AllowAlways,
    RejectOnce,
}

impl PermissionOutcome {
    pub const fn option_id(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow-once",
            Self::AllowAlways => "allow-always",
            Self::RejectOnce => "reject-once",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub option_id: String,
    pub name: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: Value,
    pub session_id: Option<String>,
    pub tool_call: Option<Value>,
    pub options: Vec<PermissionOption>,
    pub raw_params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    AgentMessageChunk { text: String },
    ToolCall { raw: Value },
    ToolCallUpdate { raw: Value },
    Plan { raw: Value },
    UsageUpdate { raw: Value },
    PermissionRequest { request: PermissionRequest },
    CursorExtension { method: String, params: Value },
    SessionUpdate { update_type: String, raw: Value },
    Completed { stop_reason: Option<String> },
    ProviderError { message: String },
}

pub trait RuntimeEventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);
}

impl<F> RuntimeEventSink for F
where
    F: Fn(RuntimeEvent) + Send + Sync,
{
    fn emit(&self, event: RuntimeEvent) {
        self(event);
    }
}

pub trait RuntimeInteractionHandler: Send + Sync {
    fn decide_permission(&self, request: &PermissionRequest) -> PermissionOutcome;

    fn handle_cursor_extension(&self, _method: &str, _params: &Value) -> Option<Value> {
        None
    }
}

#[derive(Default)]
pub struct RejectingInteractionHandler;

impl RuntimeInteractionHandler for RejectingInteractionHandler {
    fn decide_permission(&self, _request: &PermissionRequest) -> PermissionOutcome {
        PermissionOutcome::RejectOnce
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_identity_is_explicit_and_round_trips() {
        assert_eq!("openai".parse::<ProviderId>().unwrap(), ProviderId::OpenAi);
        assert_eq!("codex".parse::<ProviderId>().unwrap(), ProviderId::OpenAi);
        assert_eq!("cursor".parse::<ProviderId>().unwrap(), ProviderId::Cursor);
        assert!("gpt-5".parse::<ProviderId>().is_err());
    }

    #[test]
    fn model_id_never_infers_provider_from_model_name() {
        let model = RuntimeModelId::new(ProviderId::Cursor, "gpt-5").unwrap();
        assert_eq!(model.provider, ProviderId::Cursor);
        assert_eq!(model.model, "gpt-5");
    }
}
