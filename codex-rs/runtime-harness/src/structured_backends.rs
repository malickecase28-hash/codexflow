use crate::ConstraintProviderError;
use crate::OutputConstraint;
use crate::OutputConstraintKind;
use crate::PreparedConstraint;
use crate::StructuredGenerationCapabilities;
use crate::StructuredGenerationProvider;
use crate::StructuredGenerationRequest;

#[derive(Debug, Clone, Default)]
pub struct VllmStructuredOutputProvider;

impl StructuredGenerationProvider for VllmStructuredOutputProvider {
    fn provider_id(&self) -> &str {
        "vllm"
    }

    fn capabilities(&self) -> StructuredGenerationCapabilities {
        StructuredGenerationCapabilities {
            json_schema: true,
            ebnf: true,
            ..Default::default()
        }
    }

    fn prepare(
        &self,
        request: &StructuredGenerationRequest,
    ) -> Result<PreparedConstraint, ConstraintProviderError> {
        let payload = match &request.constraint {
            OutputConstraint::JsonSchema { schema } => serde_json::json!({
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "codexflow_output",
                        "schema": schema
                    }
                }
            }),
            OutputConstraint::Ebnf { grammar } => serde_json::json!({
                "structured_outputs": {
                    "grammar": grammar
                }
            }),
            other => return Err(ConstraintProviderError::Unsupported(other.kind())),
        };
        Ok(PreparedConstraint { payload })
    }
}

/// OpenAI-style request fragment for servers backed by XGrammar.
///
/// XGrammar's current structural-tag API can express an EBNF grammar as a
/// `grammar` format object. JSON Schema uses the standard OpenAI-style
/// `response_format`. The existing `StructuralTags { tags }` harness variant is
/// intentionally not advertised here because a list of strings is not rich
/// enough to represent XGrammar's recursive structural-tag format safely.
#[derive(Debug, Clone, Default)]
pub struct XGrammarOpenAiProvider;

impl StructuredGenerationProvider for XGrammarOpenAiProvider {
    fn provider_id(&self) -> &str {
        "xgrammar-openai"
    }

    fn capabilities(&self) -> StructuredGenerationCapabilities {
        StructuredGenerationCapabilities {
            json_schema: true,
            ebnf: true,
            ..Default::default()
        }
    }

    fn prepare(
        &self,
        request: &StructuredGenerationRequest,
    ) -> Result<PreparedConstraint, ConstraintProviderError> {
        let payload = match &request.constraint {
            OutputConstraint::JsonSchema { schema } => serde_json::json!({
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "codexflow_output",
                        "schema": schema
                    }
                }
            }),
            OutputConstraint::Ebnf { grammar } => serde_json::json!({
                "response_format": {
                    "type": "structural_tag",
                    "format": {
                        "type": "grammar",
                        "grammar": grammar
                    }
                }
            }),
            other => return Err(ConstraintProviderError::Unsupported(other.kind())),
        };
        Ok(PreparedConstraint { payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn json_request() -> StructuredGenerationRequest {
        StructuredGenerationRequest {
            prompt: "return JSON".to_string(),
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

    fn grammar_request() -> StructuredGenerationRequest {
        StructuredGenerationRequest {
            prompt: "return yes or no".to_string(),
            constraint: OutputConstraint::Ebnf {
                grammar: "root ::= (\"yes\" | \"no\")".to_string(),
            },
            metadata: Value::Null,
        }
    }

    #[test]
    fn vllm_json_schema_uses_response_format() {
        let prepared = VllmStructuredOutputProvider
            .prepare(&json_request())
            .unwrap();
        assert_eq!(prepared.payload["response_format"]["type"], "json_schema");
        assert_eq!(
            prepared.payload["response_format"]["json_schema"]["name"],
            "codexflow_output"
        );
    }

    #[test]
    fn vllm_grammar_uses_structured_outputs() {
        let prepared = VllmStructuredOutputProvider
            .prepare(&grammar_request())
            .unwrap();
        assert_eq!(
            prepared.payload["structured_outputs"]["grammar"],
            "root ::= (\"yes\" | \"no\")"
        );
    }

    #[test]
    fn xgrammar_ebnf_uses_grammar_structural_format() {
        let prepared = XGrammarOpenAiProvider
            .prepare(&grammar_request())
            .unwrap();
        assert_eq!(prepared.payload["response_format"]["type"], "structural_tag");
        assert_eq!(
            prepared.payload["response_format"]["format"]["type"],
            "grammar"
        );
    }

    #[test]
    fn adapters_do_not_overstate_structural_tag_support() {
        assert!(!VllmStructuredOutputProvider
            .capabilities()
            .supports(OutputConstraintKind::StructuralTags));
        assert!(!XGrammarOpenAiProvider
            .capabilities()
            .supports(OutputConstraintKind::StructuralTags));
    }
}
