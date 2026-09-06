use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputConstraintKind {
    JsonSchema,
    Ebnf,
    Lark,
    StructuralTags,
    ToolCall,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputConstraint {
    JsonSchema { schema: Value },
    Ebnf { grammar: String },
    Lark { grammar: String },
    StructuralTags { tags: Vec<String> },
    ToolCall { schema: Value },
}

impl OutputConstraint {
    pub const fn kind(&self) -> OutputConstraintKind {
        match self {
            Self::JsonSchema { .. } => OutputConstraintKind::JsonSchema,
            Self::Ebnf { .. } => OutputConstraintKind::Ebnf,
            Self::Lark { .. } => OutputConstraintKind::Lark,
            Self::StructuralTags { .. } => OutputConstraintKind::StructuralTags,
            Self::ToolCall { .. } => OutputConstraintKind::ToolCall,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StructuredGenerationCapabilities {
    pub json_schema: bool,
    pub ebnf: bool,
    pub lark: bool,
    pub structural_tags: bool,
    pub tool_calling: bool,
}

impl StructuredGenerationCapabilities {
    pub const fn supports(self, kind: OutputConstraintKind) -> bool {
        match kind {
            OutputConstraintKind::JsonSchema => self.json_schema,
            OutputConstraintKind::Ebnf => self.ebnf,
            OutputConstraintKind::Lark => self.lark,
            OutputConstraintKind::StructuralTags => self.structural_tags,
            OutputConstraintKind::ToolCall => self.tool_calling,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredGenerationRequest {
    pub prompt: String,
    pub constraint: OutputConstraint,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedConstraint {
    /// Backend-specific request fragment. The harness treats this as opaque and
    /// records the backend id that produced it.
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintProviderError {
    Unsupported(OutputConstraintKind),
    Failed(String),
}

impl fmt::Display for ConstraintProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(kind) => write!(f, "constraint kind {kind:?} is unsupported"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConstraintProviderError {}

/// Adapter seam for XGrammar, SGLang/vLLM structured-generation engines, or
/// provider-native constrained decoding.
///
/// Implementations translate a CodexFlow constraint into the backend's request
/// shape. They do not need to own model routing or the agent loop.
pub trait StructuredGenerationProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn capabilities(&self) -> StructuredGenerationCapabilities;

    fn prepare(
        &self,
        request: &StructuredGenerationRequest,
    ) -> Result<PreparedConstraint, ConstraintProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintAttemptStatus {
    Prepared,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintAttempt {
    pub provider: String,
    pub status: ConstraintAttemptStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintRoute {
    pub provider: String,
    pub prepared: PreparedConstraint,
    pub attempts: Vec<ConstraintAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintRoutingError {
    pub kind: OutputConstraintKind,
    pub attempts: Vec<ConstraintAttempt>,
}

impl fmt::Display for ConstraintRoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "no structured-generation provider prepared {:?}; {} attempt(s) recorded",
            self.kind,
            self.attempts.len()
        )
    }
}

impl std::error::Error for ConstraintRoutingError {}

#[derive(Default)]
pub struct StructuredGenerationRouter {
    providers: Vec<Box<dyn StructuredGenerationProvider>>,
}

impl StructuredGenerationRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<P>(&mut self, provider: P)
    where
        P: StructuredGenerationProvider + 'static,
    {
        self.providers.push(Box::new(provider));
    }

    /// Choose the first provider that both advertises and successfully prepares
    /// the requested constraint, preserving evidence for every fallback.
    pub fn route(
        &self,
        request: &StructuredGenerationRequest,
    ) -> Result<ConstraintRoute, ConstraintRoutingError> {
        let kind = request.constraint.kind();
        let mut attempts = Vec::with_capacity(self.providers.len());

        for provider in &self.providers {
            if !provider.capabilities().supports(kind) {
                attempts.push(ConstraintAttempt {
                    provider: provider.provider_id().to_string(),
                    status: ConstraintAttemptStatus::Unsupported,
                    detail: Some(format!("provider does not advertise {kind:?}")),
                });
                continue;
            }

            match provider.prepare(request) {
                Ok(prepared) => {
                    let provider_id = provider.provider_id().to_string();
                    attempts.push(ConstraintAttempt {
                        provider: provider_id.clone(),
                        status: ConstraintAttemptStatus::Prepared,
                        detail: None,
                    });
                    return Ok(ConstraintRoute {
                        provider: provider_id,
                        prepared,
                        attempts,
                    });
                }
                Err(ConstraintProviderError::Unsupported(error_kind)) => {
                    attempts.push(ConstraintAttempt {
                        provider: provider.provider_id().to_string(),
                        status: ConstraintAttemptStatus::Unsupported,
                        detail: Some(format!("provider rejected {error_kind:?}")),
                    });
                }
                Err(ConstraintProviderError::Failed(message)) => {
                    attempts.push(ConstraintAttempt {
                        provider: provider.provider_id().to_string(),
                        status: ConstraintAttemptStatus::Failed,
                        detail: Some(message),
                    });
                }
            }
        }

        Err(ConstraintRoutingError { kind, attempts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Provider {
        id: &'static str,
        capabilities: StructuredGenerationCapabilities,
        fail: bool,
    }

    impl StructuredGenerationProvider for Provider {
        fn provider_id(&self) -> &str {
            self.id
        }

        fn capabilities(&self) -> StructuredGenerationCapabilities {
            self.capabilities
        }

        fn prepare(
            &self,
            request: &StructuredGenerationRequest,
        ) -> Result<PreparedConstraint, ConstraintProviderError> {
            if self.fail {
                return Err(ConstraintProviderError::Failed(
                    "backend preparation failed".to_string(),
                ));
            }
            Ok(PreparedConstraint {
                payload: serde_json::json!({
                    "provider": self.id,
                    "kind": format!("{:?}", request.constraint.kind()),
                }),
            })
        }
    }

    fn json_request() -> StructuredGenerationRequest {
        StructuredGenerationRequest {
            prompt: "return a result".to_string(),
            constraint: OutputConstraint::JsonSchema {
                schema: serde_json::json!({
                    "type": "object",
                    "required": ["ok"],
                    "properties": {"ok": {"type": "boolean"}}
                }),
            },
            metadata: Value::Null,
        }
    }

    #[test]
    fn capability_routing_skips_incompatible_backends() {
        let mut router = StructuredGenerationRouter::new();
        router.push(Provider {
            id: "plain-local-model",
            capabilities: StructuredGenerationCapabilities::default(),
            fail: false,
        });
        router.push(Provider {
            id: "xgrammar",
            capabilities: StructuredGenerationCapabilities {
                json_schema: true,
                ..Default::default()
            },
            fail: false,
        });

        let route = router.route(&json_request()).unwrap();

        assert_eq!(route.provider, "xgrammar");
        assert_eq!(route.attempts.len(), 2);
        assert_eq!(
            route.attempts[0].status,
            ConstraintAttemptStatus::Unsupported
        );
        assert_eq!(route.attempts[1].status, ConstraintAttemptStatus::Prepared);
    }

    #[test]
    fn preparation_failure_falls_through_with_evidence() {
        let capabilities = StructuredGenerationCapabilities {
            json_schema: true,
            ..Default::default()
        };
        let mut router = StructuredGenerationRouter::new();
        router.push(Provider {
            id: "broken-xgrammar",
            capabilities,
            fail: true,
        });
        router.push(Provider {
            id: "provider-native-schema",
            capabilities,
            fail: false,
        });

        let route = router.route(&json_request()).unwrap();

        assert_eq!(route.provider, "provider-native-schema");
        assert_eq!(route.attempts[0].status, ConstraintAttemptStatus::Failed);
        assert_eq!(route.attempts[1].status, ConstraintAttemptStatus::Prepared);
    }

    #[test]
    fn unsupported_constraint_fails_closed_instead_of_prompting_for_json() {
        let mut router = StructuredGenerationRouter::new();
        router.push(Provider {
            id: "plain-local-model",
            capabilities: StructuredGenerationCapabilities::default(),
            fail: false,
        });

        let error = router.route(&json_request()).unwrap_err();

        assert_eq!(error.kind, OutputConstraintKind::JsonSchema);
        assert_eq!(error.attempts.len(), 1);
        assert_eq!(
            error.attempts[0].status,
            ConstraintAttemptStatus::Unsupported
        );
    }
}
