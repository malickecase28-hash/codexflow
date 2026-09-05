use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

const REPLAY_SCHEMA: &str = "codexflow.recovery-replay.v1";
const MAX_REPLAY_HISTORY: usize = 1_000;

#[derive(Debug, Serialize)]
pub(super) struct RecoveryReplay {
    schema: &'static str,
    history_index: usize,
    offset_from_latest: usize,
    recorded_at: String,
    matches_current_policy: bool,
    changed_fields: Vec<String>,
    recorded_decision: Value,
    recomputed_decision: Value,
}

pub(super) fn replay(project_root: &Path, offset_from_latest: usize) -> Result<RecoveryReplay> {
    let events = super::recovery_ledger::history(project_root, MAX_REPLAY_HISTORY)?;
    if events.is_empty() {
        bail!("no recovery events are available to replay");
    }
    if offset_from_latest >= events.len() {
        bail!(
            "replay offset {offset_from_latest} exceeds available recovery history of {} records",
            events.len()
        );
    }

    let history_index = events.len() - 1 - offset_from_latest;
    let event = &events[history_index];
    let recorded_at = event
        .get("recorded_at")
        .and_then(Value::as_str)
        .context("recovery event is missing recorded_at")?
        .to_string();
    let recorded_decision = event
        .get("decision")
        .and_then(Value::as_object)
        .context("recovery event is missing decision object")?;

    let failure = required_str(recorded_decision.get("failure_class"), "failure_class")?;
    let attempt = recorded_decision
        .get("attempt")
        .and_then(Value::as_u64)
        .context("recovery decision has invalid attempt")?;
    let attempt = u32::try_from(attempt).context("recovery decision attempt exceeds u32")?;
    let current_profile =
        required_str(recorded_decision.get("current_profile"), "current_profile")?;
    let detail = match recorded_decision.get("detail") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Null) | None => None,
        Some(_) => bail!("recovery decision has invalid detail"),
    };

    let recomputed = super::resolve_recovery(
        project_root,
        failure,
        attempt,
        Some(current_profile),
        detail,
    )?;
    let recomputed_decision = serde_json::to_value(recomputed)?;
    let recorded_decision = Value::Object(recorded_decision.clone());
    let changed_fields = changed_fields(&recorded_decision, &recomputed_decision)?;

    Ok(RecoveryReplay {
        schema: REPLAY_SCHEMA,
        history_index,
        offset_from_latest,
        recorded_at,
        matches_current_policy: changed_fields.is_empty(),
        changed_fields,
        recorded_decision,
        recomputed_decision,
    })
}

fn required_str<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str> {
    value
        .and_then(Value::as_str)
        .with_context(|| format!("recovery decision has invalid {field}"))
}

fn changed_fields(recorded: &Value, recomputed: &Value) -> Result<Vec<String>> {
    let recorded = recorded
        .as_object()
        .context("recorded recovery decision is not an object")?;
    let recomputed = recomputed
        .as_object()
        .context("recomputed recovery decision is not an object")?;
    let keys: BTreeSet<&str> = recorded
        .keys()
        .chain(recomputed.keys())
        .map(String::as_str)
        .collect();
    Ok(keys
        .into_iter()
        .filter(|key| recorded.get(*key) != recomputed.get(*key))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn append_reasoning_event(root: &Path) {
        let decision = super::super::resolve_recovery(
            root,
            super::super::FAILURE_REASONING,
            2,
            Some(super::super::PROFILE_FAST),
            Some("same hypothesis failed twice".to_string()),
        )
        .expect("recovery decision");
        super::super::recovery_ledger::append(root, &decision).expect("append recovery event");
    }

    #[test]
    fn replay_matches_unchanged_policy() {
        let temp = tempfile::tempdir().expect("tempdir");
        append_reasoning_event(temp.path());
        let replay = replay(temp.path(), 0).expect("replay");
        assert!(replay.matches_current_policy);
        assert!(replay.changed_fields.is_empty());
        assert_eq!(replay.offset_from_latest, 0);
        assert_eq!(replay.history_index, 0);
    }

    #[test]
    fn replay_detects_policy_drift() {
        let temp = tempfile::tempdir().expect("tempdir");
        append_reasoning_event(temp.path());

        let mut policy = super::super::RoutingPolicy::default();
        policy.escalation_failure_threshold = 5;
        let path = super::super::policy_path(temp.path());
        fs::create_dir_all(path.parent().expect("policy parent")).expect("create policy parent");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&policy).expect("serialize policy"),
        )
        .expect("write policy");

        let replay = replay(temp.path(), 0).expect("replay");
        assert!(!replay.matches_current_policy);
        assert!(
            replay
                .changed_fields
                .iter()
                .any(|field| field == "next_profile")
        );
        assert!(
            replay
                .changed_fields
                .iter()
                .any(|field| field == "verification_depth")
        );
    }

    #[test]
    fn replay_rejects_out_of_range_offset() {
        let temp = tempfile::tempdir().expect("tempdir");
        append_reasoning_event(temp.path());
        assert!(replay(temp.path(), 1).is_err());
    }
}
