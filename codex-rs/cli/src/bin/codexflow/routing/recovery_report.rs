use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const REPORT_SCHEMA: &str = "codexflow.recovery-report.v1";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(super) struct RecoveryReport {
    schema: &'static str,
    records: usize,
    latest_recorded_at: Option<String>,
    failure_counts: BTreeMap<String, u64>,
    profile_transitions: BTreeMap<String, u64>,
    verification_depth_counts: BTreeMap<String, u64>,
    retry_allowed: u64,
    retry_blocked: u64,
    strategy_changes: u64,
    retrieval_expansions: u64,
    rollback_recommendations: u64,
    human_gates: u64,
    escalations: u64,
}

pub(super) fn build(project_root: &Path, limit: usize) -> Result<RecoveryReport> {
    let events = super::recovery_ledger::history(project_root, limit)?;
    build_from_events(&events)
}

fn build_from_events(events: &[Value]) -> Result<RecoveryReport> {
    let mut failure_counts = BTreeMap::new();
    let mut profile_transitions = BTreeMap::new();
    let mut verification_depth_counts = BTreeMap::new();
    let mut retry_allowed = 0u64;
    let mut retry_blocked = 0u64;
    let mut strategy_changes = 0u64;
    let mut retrieval_expansions = 0u64;
    let mut rollback_recommendations = 0u64;
    let mut human_gates = 0u64;
    let mut escalations = 0u64;

    for (index, event) in events.iter().enumerate() {
        let decision = event
            .get("decision")
            .and_then(Value::as_object)
            .with_context(|| format!("recovery event {} is missing decision object", index + 1))?;
        let failure_class = required_str(decision.get("failure_class"), "failure_class", index)?;
        let current_profile =
            required_profile(decision.get("current_profile"), "current_profile", index)?;
        let next_profile = required_profile(decision.get("next_profile"), "next_profile", index)?;
        let verification_depth = required_str(
            decision.get("verification_depth"),
            "verification_depth",
            index,
        )?;

        increment(&mut failure_counts, failure_class);
        increment(
            &mut profile_transitions,
            &format!("{current_profile}->{next_profile}"),
        );
        increment(&mut verification_depth_counts, verification_depth);

        if required_bool(decision.get("retry_allowed"), "retry_allowed", index)? {
            retry_allowed = retry_allowed.saturating_add(1);
        } else {
            retry_blocked = retry_blocked.saturating_add(1);
        }
        if required_bool(decision.get("strategy_change"), "strategy_change", index)? {
            strategy_changes = strategy_changes.saturating_add(1);
        }
        if required_bool(
            decision.get("additional_retrieval"),
            "additional_retrieval",
            index,
        )? {
            retrieval_expansions = retrieval_expansions.saturating_add(1);
        }
        if required_bool(
            decision.get("rollback_recommended"),
            "rollback_recommended",
            index,
        )? {
            rollback_recommendations = rollback_recommendations.saturating_add(1);
        }
        if required_bool(decision.get("human_approval"), "human_approval", index)? {
            human_gates = human_gates.saturating_add(1);
        }
        if profile_rank(next_profile) > profile_rank(current_profile) {
            escalations = escalations.saturating_add(1);
        }
    }

    let latest_recorded_at = events
        .last()
        .and_then(|event| event.get("recorded_at"))
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(RecoveryReport {
        schema: REPORT_SCHEMA,
        records: events.len(),
        latest_recorded_at,
        failure_counts,
        profile_transitions,
        verification_depth_counts,
        retry_allowed,
        retry_blocked,
        strategy_changes,
        retrieval_expansions,
        rollback_recommendations,
        human_gates,
        escalations,
    })
}

fn required_str<'a>(value: Option<&'a Value>, field: &str, index: usize) -> Result<&'a str> {
    value
        .and_then(Value::as_str)
        .with_context(|| format!("recovery event {} has invalid {field}", index + 1))
}

fn required_bool(value: Option<&Value>, field: &str, index: usize) -> Result<bool> {
    value
        .and_then(Value::as_bool)
        .with_context(|| format!("recovery event {} has invalid {field}", index + 1))
}

fn required_profile<'a>(value: Option<&'a Value>, field: &str, index: usize) -> Result<&'a str> {
    let profile = required_str(value, field, index)?;
    profile_rank_checked(profile).with_context(|| {
        format!(
            "recovery event {} has invalid {field} {profile:?}",
            index + 1
        )
    })?;
    Ok(profile)
}

fn profile_rank_checked(profile: &str) -> Result<u8> {
    match profile {
        "fast" => Ok(0),
        "balanced" => Ok(1),
        "deep" => Ok(2),
        "critical" => Ok(3),
        _ => bail!("unknown recovery profile {profile:?}"),
    }
}

fn profile_rank(profile: &str) -> u8 {
    profile_rank_checked(profile).expect("profile validated before rank comparison")
}

fn increment(map: &mut BTreeMap<String, u64>, key: &str) {
    let entry = map.entry(key.to_string()).or_default();
    *entry = entry.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        failure: &str,
        current: &str,
        next: &str,
        retry_allowed: bool,
        strategy_change: bool,
        retrieval: bool,
        rollback: bool,
        human: bool,
        verification: &str,
    ) -> Value {
        serde_json::json!({
            "schema": "codexflow.recovery-event.v1",
            "recorded_at": "2026-09-04T00:00:00.000Z",
            "decision": {
                "failure_class": failure,
                "current_profile": current,
                "next_profile": next,
                "retry_allowed": retry_allowed,
                "strategy_change": strategy_change,
                "additional_retrieval": retrieval,
                "rollback_recommended": rollback,
                "human_approval": human,
                "verification_depth": verification,
            }
        })
    }

    #[test]
    fn aggregates_recovery_trajectory_metrics() {
        let events = vec![
            event(
                "test", "fast", "balanced", true, true, false, true, false, "standard",
            ),
            event(
                "permission",
                "balanced",
                "balanced",
                false,
                false,
                false,
                false,
                true,
                "standard",
            ),
        ];
        let report = build_from_events(&events).expect("report");
        assert_eq!(report.records, 2);
        assert_eq!(report.retry_allowed, 1);
        assert_eq!(report.retry_blocked, 1);
        assert_eq!(report.strategy_changes, 1);
        assert_eq!(report.rollback_recommendations, 1);
        assert_eq!(report.human_gates, 1);
        assert_eq!(report.escalations, 1);
        assert_eq!(report.failure_counts.get("test"), Some(&1));
        assert_eq!(report.profile_transitions.get("fast->balanced"), Some(&1));
    }

    #[test]
    fn rejects_invalid_profile_in_persisted_event() {
        let events = vec![event(
            "reasoning",
            "unknown",
            "critical",
            true,
            true,
            false,
            false,
            true,
            "exhaustive",
        )];
        assert!(build_from_events(&events).is_err());
    }
}
