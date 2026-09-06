use crate::ToolPolicy;
use crate::ToolPolicyDecision;
use crate::ToolPolicyRequest;
use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

#[derive(Debug, Clone)]
pub struct OpaCliPolicy {
    id: String,
    executable: PathBuf,
    working_directory: PathBuf,
    data_paths: Vec<PathBuf>,
    query: String,
    timeout: String,
}

impl OpaCliPolicy {
    pub fn new(
        id: impl Into<String>,
        executable: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
        query: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            executable: executable.into(),
            working_directory: working_directory.into(),
            data_paths: Vec::new(),
            query: query.into(),
            timeout: "2s".to_string(),
        }
    }

    pub fn with_data_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.data_paths = paths.into_iter().collect();
        self
    }

    pub fn with_timeout(mut self, timeout: impl Into<String>) -> Self {
        self.timeout = timeout.into();
        self
    }

    fn evaluate_opa(&self, request: &ToolPolicyRequest) -> Result<ToolPolicyDecision, String> {
        if self.query.trim().is_empty() {
            return Err("OPA policy query cannot be empty".to_string());
        }
        if self.timeout.trim().is_empty() {
            return Err("OPA policy timeout cannot be empty".to_string());
        }

        let mut command = Command::new(&self.executable);
        command
            .current_dir(&self.working_directory)
            .arg("eval")
            .arg("--format=json")
            .arg("--stdin-input")
            .arg("--strict")
            .arg("--timeout")
            .arg(&self.timeout);
        for path in &self.data_paths {
            command.arg("--data").arg(path);
        }
        command
            .arg(&self.query)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to execute OPA at {}: {error}",
                self.executable.display()
            )
        })?;
        let input = serde_json::to_vec(request)
            .map_err(|error| format!("failed to serialize OPA input: {error}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "OPA stdin pipe was unavailable".to_string())?;
        stdin
            .write_all(&input)
            .map_err(|error| format!("failed to write OPA input: {error}"))?;
        drop(stdin);

        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed waiting for OPA: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "OPA exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let response: OpaEvalResponse = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid OPA JSON output: {error}"))?;
        let Some(value) = response
            .result
            .first()
            .and_then(|result| result.expressions.first())
            .map(|expression| &expression.value)
        else {
            return Ok(ToolPolicyDecision::Abstain);
        };
        parse_policy_value(value)
    }
}

impl ToolPolicy for OpaCliPolicy {
    fn policy_id(&self) -> &str {
        &self.id
    }

    fn evaluate(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        self.evaluate_opa(request)
            .unwrap_or_else(|error| ToolPolicyDecision::Deny {
                reason: format!("OPA policy evaluation failed closed: {error}"),
            })
    }
}

#[derive(Debug, Deserialize)]
struct OpaEvalResponse {
    #[serde(default)]
    result: Vec<OpaResult>,
}

#[derive(Debug, Deserialize)]
struct OpaResult {
    #[serde(default)]
    expressions: Vec<OpaExpression>,
}

#[derive(Debug, Deserialize)]
struct OpaExpression {
    value: Value,
}

fn parse_policy_value(value: &Value) -> Result<ToolPolicyDecision, String> {
    match value {
        Value::Bool(true) => Ok(ToolPolicyDecision::Allow),
        Value::Bool(false) => Ok(ToolPolicyDecision::Deny {
            reason: "OPA returned false".to_string(),
        }),
        Value::String(decision) => parse_named_decision(decision, None),
        Value::Object(object) => {
            let decision = object
                .get("decision")
                .and_then(Value::as_str)
                .ok_or_else(|| "OPA decision object must contain string field 'decision'".to_string())?;
            let reason = object
                .get("reason")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            parse_named_decision(decision, reason)
        }
        other => Err(format!(
            "unsupported OPA decision value {}; expected bool, string, or object",
            other
        )),
    }
}

fn parse_named_decision(
    decision: &str,
    reason: Option<String>,
) -> Result<ToolPolicyDecision, String> {
    match decision.trim().to_ascii_lowercase().as_str() {
        "allow" => Ok(ToolPolicyDecision::Allow),
        "confirm" => Ok(ToolPolicyDecision::Confirm {
            reason: reason.unwrap_or_else(|| "OPA requires user confirmation".to_string()),
        }),
        "deny" => Ok(ToolPolicyDecision::Deny {
            reason: reason.unwrap_or_else(|| "OPA denied tool invocation".to_string()),
        }),
        "abstain" => Ok(ToolPolicyDecision::Abstain),
        unknown => Err(format!("unknown OPA decision '{unknown}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_decisions_are_supported() {
        assert_eq!(
            parse_policy_value(&Value::Bool(true)).unwrap(),
            ToolPolicyDecision::Allow
        );
        assert!(matches!(
            parse_policy_value(&Value::Bool(false)).unwrap(),
            ToolPolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn object_decision_preserves_reason() {
        let decision = parse_policy_value(&serde_json::json!({
            "decision": "confirm",
            "reason": "network access requires review"
        }))
        .unwrap();

        assert_eq!(
            decision,
            ToolPolicyDecision::Confirm {
                reason: "network access requires review".to_string()
            }
        );
    }

    #[test]
    fn undefined_eval_result_becomes_abstain() {
        let response: OpaEvalResponse = serde_json::from_str(r#"{"result":[]}"#).unwrap();
        assert!(response.result.is_empty());
    }

    #[test]
    fn malformed_decisions_fail_instead_of_allowing() {
        assert!(parse_policy_value(&serde_json::json!({"decision":"maybe"})).is_err());
        assert!(parse_policy_value(&serde_json::json!(["allow"])).is_err());
    }
}
