use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(super) struct HandoffContext {
    pub accomplished: Vec<String>,
    pub remaining_work: Vec<String>,
    pub failures: Vec<String>,
    pub relevant_files: Vec<String>,
    pub decisions: Vec<String>,
    pub rationale: Option<String>,
    pub restart_commands: Vec<String>,
    pub explicit_next_action: Option<String>,
}

impl HandoffContext {
    pub(super) fn from_latest(latest_handoff: Option<&Value>) -> Self {
        let Some(handoff) = latest_handoff.and_then(Value::as_object) else {
            return Self::default();
        };
        Self {
            accomplished: string_array(handoff.get("accomplished")),
            remaining_work: string_array(handoff.get("remaining_work")),
            failures: string_array(handoff.get("failures")),
            relevant_files: string_array(handoff.get("relevant_files")),
            decisions: string_array(handoff.get("decisions")),
            rationale: optional_string(handoff.get("rationale")),
            restart_commands: string_array(handoff.get("restart_commands")),
            explicit_next_action: optional_string(handoff.get("next_action")),
        }
    }

    pub(super) fn next_action_hint(&self) -> Option<&str> {
        self.explicit_next_action
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn structured_handoff_is_extracted_without_inference() {
        let handoff = json!({
            "accomplished": ["compiled runtime", "  recovery eval passed  "],
            "remaining_work": ["publish edge release"],
            "failures": ["windows x64 cold build cancelled"],
            "relevant_files": ["codex-rs/cli/src/bin/codexflow/runtime.rs"],
            "decisions": ["ownership changes only through agent-set"],
            "rationale": "leases are authority",
            "restart_commands": ["cargo test -p codex-cli --bin codexflow"],
            "next_action": "rerun six-target release"
        });
        let context = HandoffContext::from_latest(Some(&handoff));
        assert_eq!(context.accomplished[1], "recovery eval passed");
        assert_eq!(context.remaining_work, ["publish edge release"]);
        assert_eq!(context.next_action_hint(), Some("rerun six-target release"));
    }

    #[test]
    fn legacy_handoff_has_empty_structured_context() {
        let handoff = json!({
            "from": "worker-a",
            "to": "worker-b",
            "summary": "legacy summary",
            "refs": ["src/lib.rs"],
            "at": "2026-09-05T00:00:00Z"
        });
        assert_eq!(HandoffContext::from_latest(Some(&handoff)), HandoffContext::default());
    }
}
