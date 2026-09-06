use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::BTreeMap;

const TOOL_RESULT_FORMAT_VERSION: u32 = 1;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultProvenance {
    pub tool_name: String,
    pub tool_call_id: Option<String>,
    pub provider: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionRule {
    pub pointer: String,
    pub label: String,
    pub priority: u8,
    pub required: bool,
    pub abnormal_when: Option<ValueMatcher>,
}

impl ProjectionRule {
    pub fn new(pointer: impl Into<String>, label: impl Into<String>, priority: u8) -> Self {
        Self {
            pointer: pointer.into(),
            label: label.into(),
            priority,
            required: false,
            abnormal_when: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn abnormal_when(mut self, matcher: ValueMatcher) -> Self {
        self.abnormal_when = Some(matcher);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueMatcher {
    Equals(Value),
    Missing,
    EmptyString,
    False,
    True,
}

impl ValueMatcher {
    fn matches(&self, value: Option<&Value>) -> bool {
        match self {
            Self::Equals(expected) => value == Some(expected),
            Self::Missing => value.is_none() || value == Some(&Value::Null),
            Self::EmptyString => value.and_then(Value::as_str).is_some_and(str::is_empty),
            Self::False => value == Some(&Value::Bool(false)),
            Self::True => value == Some(&Value::Bool(true)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultFact {
    pub label: String,
    pub value: Value,
    pub priority: u8,
    pub abnormal: bool,
    pub source_pointer: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultView {
    pub facts: Vec<ToolResultFact>,
    pub missing_required: Vec<String>,
    pub omitted_facts: usize,
    pub rendered: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEnvelope {
    pub format_version: u32,
    pub provenance: ToolResultProvenance,
    pub fingerprint: String,
    pub view: ToolResultView,
    raw: Value,
}

impl ToolResultEnvelope {
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    pub fn rehydrate_raw(&self) -> Value {
        self.raw.clone()
    }
}

#[derive(Debug, Clone)]
pub struct JsonProjectionTransformer {
    rules: Vec<ProjectionRule>,
    id_names: BTreeMap<String, String>,
    max_bytes: usize,
}

impl JsonProjectionTransformer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            rules: Vec::new(),
            id_names: BTreeMap::new(),
            max_bytes,
        }
    }

    pub fn with_rules(mut self, rules: impl IntoIterator<Item = ProjectionRule>) -> Self {
        self.rules = rules.into_iter().collect();
        self
    }

    /// Resolve exact opaque string values into stable semantic names before the
    /// compact view is rendered. The raw artifact remains unchanged.
    pub fn with_id_names(
        mut self,
        names: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.id_names = names.into_iter().collect();
        self
    }

    pub fn transform(
        &self,
        provenance: ToolResultProvenance,
        raw: Value,
    ) -> Result<ToolResultEnvelope, ToolResultTransformError> {
        validate_provenance(&provenance)?;
        let mut facts = Vec::new();
        let mut missing_required = Vec::new();

        for rule in &self.rules {
            let raw_value = raw.pointer(&rule.pointer);
            let abnormal = rule
                .abnormal_when
                .as_ref()
                .is_some_and(|matcher| matcher.matches(raw_value));
            let Some(value) = raw_value else {
                if rule.required {
                    missing_required.push(rule.label.clone());
                }
                continue;
            };
            if value.is_null() {
                if rule.required {
                    missing_required.push(rule.label.clone());
                }
                continue;
            }
            facts.push(ToolResultFact {
                label: rule.label.clone(),
                value: resolve_value(value, &self.id_names),
                priority: rule.priority,
                abnormal,
                source_pointer: rule.pointer.clone(),
            });
        }

        facts.sort_by_key(|fact| (Reverse(fact.priority), fact.label.clone()));
        missing_required.sort();
        missing_required.dedup();

        let discovered = facts.len();
        let mut selected = Vec::new();
        let mut rendered = Vec::new();
        let mut used_bytes = 0usize;
        for fact in facts {
            let line = render_fact(&fact);
            let separator = if rendered.is_empty() { 0 } else { 1 };
            if used_bytes
                .saturating_add(separator)
                .saturating_add(line.len())
                > self.max_bytes
            {
                continue;
            }
            used_bytes += separator + line.len();
            rendered.push(line);
            selected.push(fact);
        }

        if !missing_required.is_empty() {
            let missing_line = format!("MISSING: {}", missing_required.join(", "));
            let separator = if rendered.is_empty() { 0 } else { 1 };
            if used_bytes
                .saturating_add(separator)
                .saturating_add(missing_line.len())
                <= self.max_bytes
            {
                rendered.push(missing_line);
            }
        }

        let fingerprint = fingerprint_raw(&raw)?;
        Ok(ToolResultEnvelope {
            format_version: TOOL_RESULT_FORMAT_VERSION,
            provenance,
            fingerprint,
            view: ToolResultView {
                omitted_facts: discovered.saturating_sub(selected.len()),
                facts: selected,
                missing_required,
                rendered: rendered.join("\n"),
            },
            raw,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolResultTransformError {
    #[error("tool result provenance requires a non-empty tool name")]
    EmptyToolName,
    #[error("tool result provenance requires a non-empty source")]
    EmptySource,
    #[error("failed to serialize raw tool result for fingerprinting: {0}")]
    Serialize(#[from] serde_json::Error),
}

fn validate_provenance(provenance: &ToolResultProvenance) -> Result<(), ToolResultTransformError> {
    if provenance.tool_name.trim().is_empty() {
        return Err(ToolResultTransformError::EmptyToolName);
    }
    if provenance.source.trim().is_empty() {
        return Err(ToolResultTransformError::EmptySource);
    }
    Ok(())
}

fn resolve_value(value: &Value, id_names: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(text) => id_names
            .get(text)
            .map(|name| Value::String(format!("{name} ({text})")))
            .unwrap_or_else(|| value.clone()),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| resolve_value(value, id_names))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), resolve_value(value, id_names)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn render_fact(fact: &ToolResultFact) -> String {
    let marker = if fact.abnormal { "! " } else { "" };
    format!("{marker}{}: {}", fact.label, compact_json(&fact.value))
}

fn compact_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn fingerprint_raw(raw: &Value) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(raw)?;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in TOOL_RESULT_FORMAT_VERSION
        .to_le_bytes()
        .into_iter()
        .chain(bytes.into_iter())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    Ok(format!("{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> ToolResultProvenance {
        ToolResultProvenance {
            tool_name: "issue_lookup".to_string(),
            tool_call_id: Some("call-1".to_string()),
            provider: Some("github".to_string()),
            source: "github issue API".to_string(),
        }
    }

    #[test]
    fn projection_drops_unrequested_metadata_and_preserves_raw() {
        let raw = serde_json::json!({
            "id": "usr_123",
            "status": "open",
            "internal_debug": {"trace": [1,2,3]},
            "assignee": {"id": "usr_123"}
        });
        let transformer = JsonProjectionTransformer::new(1024)
            .with_rules([
                ProjectionRule::new("/status", "status", 100).required(),
                ProjectionRule::new("/assignee/id", "assignee", 80),
            ])
            .with_id_names([("usr_123".to_string(), "Ari".to_string())]);

        let envelope = transformer
            .transform(provenance(), raw.clone())
            .unwrap();

        assert_eq!(envelope.rehydrate_raw(), raw);
        assert!(!envelope.view.rendered.contains("internal_debug"));
        assert!(envelope.view.rendered.contains("assignee: Ari (usr_123)"));
        assert_eq!(envelope.view.facts.len(), 2);
    }

    #[test]
    fn missing_and_abnormal_values_are_explicit() {
        let transformer = JsonProjectionTransformer::new(1024).with_rules([
            ProjectionRule::new("/healthy", "healthy", 100)
                .required()
                .abnormal_when(ValueMatcher::False),
            ProjectionRule::new("/owner", "owner", 90).required(),
        ]);

        let envelope = transformer
            .transform(provenance(), serde_json::json!({"healthy": false}))
            .unwrap();

        assert!(envelope.view.rendered.contains("! healthy: false"));
        assert!(envelope.view.rendered.contains("MISSING: owner"));
        assert_eq!(envelope.view.missing_required, vec!["owner".to_string()]);
    }

    #[test]
    fn relevance_priority_controls_budget_selection() {
        let high = ProjectionRule::new("/high", "high", 100);
        let low = ProjectionRule::new("/low", "low", 1);
        let transformer = JsonProjectionTransformer::new("high: important".len())
            .with_rules([low, high]);
        let envelope = transformer
            .transform(
                provenance(),
                serde_json::json!({"high":"important","low":"verbose low priority"}),
            )
            .unwrap();

        assert_eq!(envelope.view.facts.len(), 1);
        assert_eq!(envelope.view.facts[0].label, "high");
        assert_eq!(envelope.view.omitted_facts, 1);
    }

    #[test]
    fn fingerprint_changes_with_raw_evidence() {
        let transformer = JsonProjectionTransformer::new(1024);
        let first = transformer
            .transform(provenance(), serde_json::json!({"value":1}))
            .unwrap();
        let second = transformer
            .transform(provenance(), serde_json::json!({"value":2}))
            .unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
    }
}
