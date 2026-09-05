use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

const RUNTIME_SCHEMA: &str = "codexflow.runtime.v2";
const RESUME_SCHEMA: &str = "codexflow.resume-packet.v1";

#[derive(Debug, Args)]
pub struct ResumeArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    task: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ResumePacket {
    schema: &'static str,
    task: String,
    title: Option<String>,
    status: Option<String>,
    risk: Option<String>,
    assignee: Option<String>,
    waiting_on: Option<String>,
    used_tokens: u64,
    budget_tokens: Option<u64>,
    latest_handoff: Option<Value>,
    relevant_refs: Vec<String>,
    dependencies: Vec<DependencyStatus>,
    pending_acceptance: Vec<AcceptanceStatus>,
    blocking_gates: Vec<BlockingGate>,
    blockers: Vec<String>,
    next_action: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct DependencyStatus {
    id: String,
    status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct AcceptanceStatus {
    id: String,
    text: String,
    status: String,
    evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BlockingGate {
    name: String,
    risk: Option<String>,
    reviewer: Option<String>,
    finding: Option<String>,
}

pub fn handle(project_root: &Path, args: ResumeArgs) -> Result<()> {
    let ledger = load_ledger(project_root)?;
    let packet = build_packet(&ledger, &args.task)?;
    println!("{}", serde_json::to_string_pretty(&packet)?);
    Ok(())
}

fn load_ledger(project_root: &Path) -> Result<Value> {
    let path = project_root
        .join(".codexflow")
        .join("state")
        .join("runtime-v2.json");
    let data = fs::read_to_string(&path)
        .with_context(|| format!("read CodexFlow runtime state {}", path.display()))?;
    let ledger: Value = serde_json::from_str(&data)
        .with_context(|| format!("parse CodexFlow runtime state {}", path.display()))?;
    if ledger.get("schema").and_then(Value::as_str) != Some(RUNTIME_SCHEMA) {
        bail!("unsupported CodexFlow runtime schema in {}", path.display());
    }
    Ok(ledger)
}

fn build_packet(ledger: &Value, task_id: &str) -> Result<ResumePacket> {
    let tasks = ledger
        .get("tasks")
        .and_then(Value::as_object)
        .context("runtime state tasks is not an object")?;
    let task = tasks
        .get(task_id)
        .and_then(Value::as_object)
        .with_context(|| format!("unknown task: {task_id}"))?;

    let waiting_on = optional_string(task.get("waiting_on"));
    let acceptance = task
        .get("acceptance")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut pending_acceptance = Vec::new();
    for item in acceptance {
        let Some(item) = item.as_object() else {
            continue;
        };
        let status = optional_string(item.get("status")).unwrap_or_else(|| "pending".to_string());
        if status == "pass" {
            continue;
        }
        pending_acceptance.push(AcceptanceStatus {
            id: optional_string(item.get("id")).unwrap_or_else(|| "unknown".to_string()),
            text: optional_string(item.get("text")).unwrap_or_default(),
            status,
            evidence: string_array(item.get("evidence")),
        });
    }

    let mut blocking_gates = Vec::new();
    if let Some(gates) = task.get("gates").and_then(Value::as_object) {
        for (name, gate) in gates {
            let Some(gate) = gate.as_object() else {
                continue;
            };
            if gate.get("status").and_then(Value::as_str) != Some("block") {
                continue;
            }
            blocking_gates.push(BlockingGate {
                name: name.clone(),
                risk: optional_string(gate.get("risk")),
                reviewer: optional_string(gate.get("reviewer")),
                finding: optional_string(gate.get("finding")),
            });
        }
    }

    let mut dependencies = Vec::new();
    for dependency in string_array(task.get("depends_on")) {
        let status = tasks
            .get(&dependency)
            .and_then(Value::as_object)
            .and_then(|record| record.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("missing")
            .to_string();
        dependencies.push(DependencyStatus {
            id: dependency,
            status,
        });
    }

    let latest_handoff = task
        .get("handoffs")
        .and_then(Value::as_array)
        .and_then(|handoffs| handoffs.last())
        .filter(|handoff| handoff.is_object())
        .cloned();
    let relevant_refs = latest_handoff
        .as_ref()
        .and_then(|handoff| handoff.get("refs"))
        .map(|value| string_array(Some(value)))
        .unwrap_or_default();

    let status = optional_string(task.get("status"));
    let mut blockers = Vec::new();
    if let Some(waiting_on) = waiting_on.as_deref() {
        blockers.push(format!("waiting on {waiting_on}"));
    }
    for dependency in &dependencies {
        if dependency.status != "done" {
            blockers.push(format!(
                "dependency {} is {}",
                dependency.id, dependency.status
            ));
        }
    }
    for item in &pending_acceptance {
        blockers.push(format!(
            "acceptance {} is {}: {}",
            item.id, item.status, item.text
        ));
    }
    for gate in &blocking_gates {
        blockers.push(format!("gate {} is blocking", gate.name));
    }
    if matches!(status.as_deref(), Some("failed" | "cancelled")) {
        blockers.push(format!(
            "task status {} requires intervention",
            status.as_deref().unwrap_or_default()
        ));
    }

    let next_action = derive_next_action(
        task_id,
        status.as_deref(),
        waiting_on.as_deref(),
        &dependencies,
        &pending_acceptance,
        &blocking_gates,
    );

    Ok(ResumePacket {
        schema: RESUME_SCHEMA,
        task: task_id.to_string(),
        title: optional_string(task.get("title")),
        status,
        risk: optional_string(task.get("risk")),
        assignee: optional_string(task.get("assignee")),
        waiting_on,
        used_tokens: task.get("used_tokens").and_then(Value::as_u64).unwrap_or(0),
        budget_tokens: task.get("budget_tokens").and_then(Value::as_u64),
        latest_handoff,
        relevant_refs,
        dependencies,
        pending_acceptance,
        blocking_gates,
        blockers,
        next_action,
    })
}

fn derive_next_action(
    task_id: &str,
    status: Option<&str>,
    waiting_on: Option<&str>,
    dependencies: &[DependencyStatus],
    pending_acceptance: &[AcceptanceStatus],
    blocking_gates: &[BlockingGate],
) -> String {
    if let Some(waiting_on) = waiting_on {
        return format!("wait for {waiting_on}, then wake task {task_id}");
    }
    if let Some(dependency) = dependencies.iter().find(|dependency| dependency.status != "done") {
        return format!(
            "complete dependency {} before continuing {task_id}",
            dependency.id
        );
    }
    if let Some(item) = pending_acceptance.iter().find(|item| item.status == "fail") {
        return format!("repair failed acceptance {}: {}", item.id, item.text);
    }
    if let Some(item) = pending_acceptance.first() {
        return format!("satisfy acceptance {}: {}", item.id, item.text);
    }
    if let Some(gate) = blocking_gates.first() {
        return format!("resolve blocking gate {}", gate.name);
    }
    if status == Some("done") {
        return "task is complete; no implementation work remains".to_string();
    }
    format!("continue task {task_id} and collect deterministic verification evidence")
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(ToString::to_string)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resume_prioritizes_dependency_then_failed_acceptance() {
        let mut ledger = json!({
            "schema": RUNTIME_SCHEMA,
            "tasks": {
                "dep": { "status": "doing" },
                "work": {
                    "title": "demo",
                    "status": "doing",
                    "risk": "medium",
                    "assignee": "worker",
                    "depends_on": ["dep"],
                    "budget_tokens": 1000,
                    "used_tokens": 250,
                    "waiting_on": null,
                    "acceptance": [{
                        "id": "ac-1",
                        "text": "compiler passes",
                        "status": "fail",
                        "evidence": ["cargo check failed"]
                    }],
                    "gates": {
                        "review": {
                            "status": "block",
                            "risk": "medium",
                            "reviewer": null,
                            "finding": "pending review"
                        }
                    },
                    "handoffs": [{
                        "from": "worker-a",
                        "to": "worker-b",
                        "summary": "compiler failure isolated",
                        "refs": ["src/runtime.rs", "CI run 123"],
                        "at": "2026-09-04T00:00:00Z"
                    }]
                }
            }
        });

        let packet = build_packet(&ledger, "work").expect("resume packet");
        assert_eq!(packet.schema, RESUME_SCHEMA);
        assert_eq!(
            packet.next_action,
            "complete dependency dep before continuing work"
        );
        assert_eq!(packet.relevant_refs, ["src/runtime.rs", "CI run 123"]);
        assert_eq!(packet.pending_acceptance[0].id, "ac-1");
        assert_eq!(packet.blocking_gates[0].name, "review");

        ledger["tasks"]["dep"]["status"] = json!("done");
        let packet = build_packet(&ledger, "work").expect("resume after dependency");
        assert_eq!(
            packet.next_action,
            "repair failed acceptance ac-1: compiler passes"
        );
    }

    #[test]
    fn waiting_and_done_states_have_unambiguous_next_actions() {
        let mut ledger = json!({
            "schema": RUNTIME_SCHEMA,
            "tasks": {
                "work": {
                    "title": "demo",
                    "status": "blocked_waiting",
                    "risk": "low",
                    "depends_on": [],
                    "waiting_on": "ci-run-124",
                    "acceptance": [],
                    "gates": {},
                    "handoffs": []
                }
            }
        });
        let packet = build_packet(&ledger, "work").expect("waiting packet");
        assert_eq!(
            packet.next_action,
            "wait for ci-run-124, then wake task work"
        );

        ledger["tasks"]["work"]["waiting_on"] = Value::Null;
        ledger["tasks"]["work"]["status"] = json!("done");
        let packet = build_packet(&ledger, "work").expect("done packet");
        assert!(packet.blockers.is_empty());
        assert_eq!(
            packet.next_action,
            "task is complete; no implementation work remains"
        );
    }

    #[test]
    fn unknown_task_is_rejected() {
        let ledger = json!({ "schema": RUNTIME_SCHEMA, "tasks": {} });
        let error = build_packet(&ledger, "missing").expect_err("missing task must fail");
        assert!(error.to_string().contains("unknown task"));
    }
}
