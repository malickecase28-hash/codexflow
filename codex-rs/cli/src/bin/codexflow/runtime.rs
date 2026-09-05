use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::Utc;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

#[path = "lease.rs"]
mod lease;

const SCHEMA: &str = "codexflow.runtime.v2";
const LIVE_STATUSES: &[&str] = &["pending", "running", "idle", "blocked"];

#[derive(Debug, Args)]
pub struct RuntimeArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: RuntimeCommand,
}

#[derive(Debug, Subcommand)]
enum RuntimeCommand {
    Init,
    TaskCreate {
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "medium")]
        risk: String,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        #[arg(long = "acceptance")]
        acceptance: Vec<String>,
    },
    TaskSet {
        #[arg(long)]
        id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        risk: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        budget_tokens: Option<u64>,
    },
    TaskAcceptanceAdd {
        #[arg(long)]
        id: String,
        #[arg(long)]
        criterion: Option<String>,
        #[arg(long)]
        text: String,
    },
    TaskEvidence {
        #[arg(long)]
        id: String,
        #[arg(long)]
        criterion: String,
        #[arg(long)]
        status: String,
        #[arg(long)]
        evidence: String,
    },
    TaskComplete {
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "god")]
        actor: String,
    },
    TaskWait {
        #[arg(long)]
        id: String,
        #[arg(long = "await")]
        await_id: String,
    },
    TaskWake {
        #[arg(long)]
        id: String,
    },
    TaskList,
    AgentSet {
        #[arg(long)]
        name: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        status: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        budget_tokens: Option<u64>,
    },
    AgentHeartbeat {
        #[arg(long)]
        name: String,
        #[arg(long)]
        progress: Option<String>,
    },
    AgentAction {
        #[arg(long)]
        name: String,
        #[arg(long)]
        action: String,
        #[arg(long)]
        error: bool,
    },
    AgentTokens {
        #[arg(long)]
        name: String,
        #[arg(long)]
        add: u64,
    },
    AgentList,
    GateSet {
        #[arg(long)]
        task: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        status: String,
        #[arg(long)]
        risk: String,
        #[arg(long)]
        reviewer: Option<String>,
        #[arg(long)]
        finding: Option<String>,
    },
    HandoffAdd {
        #[arg(long)]
        task: String,
        #[arg(long = "from")]
        from_actor: String,
        #[arg(long = "to")]
        to_actor: String,
        #[arg(long)]
        summary: String,
        #[arg(long = "ref")]
        refs: Vec<String>,
        #[arg(long = "accomplished")]
        accomplished: Vec<String>,
        #[arg(long = "remaining")]
        remaining_work: Vec<String>,
        #[arg(long = "failure")]
        failures: Vec<String>,
        #[arg(long = "file")]
        relevant_files: Vec<String>,
        #[arg(long = "decision")]
        decisions: Vec<String>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long = "restart-command")]
        restart_commands: Vec<String>,
        #[arg(long = "next-action")]
        next_action: Option<String>,
    },
    Snapshot,
    Supervise {
        #[arg(long)]
        apply: bool,
        #[arg(long, default_value_t = 900)]
        stale_seconds: u64,
        #[arg(long, default_value_t = 6)]
        max_repeated_action: u32,
        #[arg(long, default_value_t = 4)]
        max_consecutive_errors: u32,
        #[arg(long, default_value_t = 5)]
        max_no_progress: u32,
        #[arg(long, default_value_t = 8)]
        max_live_agents: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeLedger {
    schema: String,
    project_root: String,
    created_at: String,
    updated_at: String,
    tasks: BTreeMap<String, TaskRecord>,
    agents: BTreeMap<String, AgentRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskRecord {
    title: String,
    status: String,
    risk: String,
    assignee: Option<String>,
    depends_on: Vec<String>,
    budget_tokens: Option<u64>,
    used_tokens: u64,
    #[serde(default)]
    waiting_on: Option<String>,
    #[serde(default)]
    acceptance: Vec<AcceptanceCriterion>,
    gates: BTreeMap<String, GateRecord>,
    handoffs: Vec<HandoffRecord>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcceptanceCriterion {
    id: String,
    text: String,
    status: String,
    evidence: Vec<String>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentRecord {
    role: String,
    status: String,
    task: Option<String>,
    budget_tokens: Option<u64>,
    used_tokens: u64,
    last_heartbeat_ms: i64,
    last_progress: Option<String>,
    no_progress_count: u32,
    last_action: Option<String>,
    repeated_action_count: u32,
    consecutive_errors: u32,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GateRecord {
    status: String,
    risk: String,
    reviewer: Option<String>,
    finding: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandoffRecord {
    from: String,
    to: String,
    summary: String,
    refs: Vec<String>,
    #[serde(default)]
    accomplished: Vec<String>,
    #[serde(default)]
    remaining_work: Vec<String>,
    #[serde(default)]
    failures: Vec<String>,
    #[serde(default)]
    relevant_files: Vec<String>,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    restart_commands: Vec<String>,
    #[serde(default)]
    next_action: Option<String>,
    at: String,
}

#[derive(Debug, Clone, Serialize)]
struct BreakerFinding {
    agent: Option<String>,
    kind: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct CompletionCheck {
    task: String,
    ready: bool,
    blockers: Vec<String>,
}

pub fn handle(project_root: &Path, args: RuntimeArgs) -> Result<()> {
    match args.command {
        RuntimeCommand::Init => {
            let ledger = load_or_init(project_root)?;
            println!("{}", serde_json::to_string_pretty(&ledger)?);
            Ok(())
        }
        RuntimeCommand::TaskCreate {
            id,
            title,
            risk,
            assignee,
            depends_on,
            acceptance,
        } => with_locked_ledger(project_root, |ledger| {
            validate_id(&id)?;
            validate_risk(&risk)?;
            if assignee.is_some() {
                bail!(
                    "task-create --assignee is disabled; assign ownership with runtime agent-set"
                );
            }
            if ledger.tasks.contains_key(&id) {
                bail!("task already exists: {id}");
            }
            for dependency in &depends_on {
                validate_id(dependency)?;
            }
            let now = now_iso();
            ledger.tasks.insert(
                id.clone(),
                TaskRecord {
                    title,
                    status: "todo".to_string(),
                    risk,
                    assignee: None,
                    depends_on,
                    budget_tokens: None,
                    used_tokens: 0,
                    waiting_on: None,
                    acceptance: acceptance_records(acceptance)?,
                    gates: BTreeMap::new(),
                    handoffs: Vec::new(),
                    updated_at: now,
                },
            );
            append_event(
                project_root,
                "task.created",
                "god",
                Some(&id),
                "task created",
            )?;
            println!("{id}");
            Ok(())
        }),
        RuntimeCommand::TaskSet {
            id,
            status,
            risk,
            assignee,
            budget_tokens,
        } => with_locked_ledger(project_root, |ledger| {
            if assignee.is_some() {
                bail!("task-set --assignee is disabled; assign ownership with runtime agent-set");
            }
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            if let Some(status) = status {
                validate_task_status(&status)?;
                if status == "blocked_waiting" {
                    bail!("use task-wait so blocked_waiting records the await id");
                }
                if status == "done" {
                    bail!("use task-complete so completion criteria and gates are checked");
                }
                if matches!(status.as_str(), "failed" | "cancelled") && task.assignee.is_some() {
                    bail!(
                        "close or fail the assigned agent with runtime agent-set before marking task {id} {status}"
                    );
                }
                task.status = status;
                task.waiting_on = None;
            }
            if let Some(risk) = risk {
                validate_risk(&risk)?;
                task.risk = risk;
            }
            if budget_tokens.is_some() {
                task.budget_tokens = budget_tokens;
            }
            task.updated_at = now_iso();
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskAcceptanceAdd {
            id,
            criterion,
            text,
        } => with_locked_ledger(project_root, |ledger| {
            validate_acceptance_text(&text)?;
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            let criterion_id =
                criterion.unwrap_or_else(|| format!("ac-{}", task.acceptance.len() + 1));
            validate_id(&criterion_id)?;
            if task.acceptance.iter().any(|item| item.id == criterion_id) {
                bail!("acceptance criterion already exists: {criterion_id}");
            }
            task.acceptance.push(AcceptanceCriterion {
                id: criterion_id.clone(),
                text,
                status: "pending".to_string(),
                evidence: Vec::new(),
                updated_at: now_iso(),
            });
            task.updated_at = now_iso();
            append_event(
                project_root,
                "task.acceptance.added",
                "god",
                Some(&id),
                &criterion_id,
            )?;
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskEvidence {
            id,
            criterion,
            status,
            evidence,
        } => with_locked_ledger(project_root, |ledger| {
            validate_acceptance_status(&status)?;
            validate_evidence(&evidence)?;
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            let item = task
                .acceptance
                .iter_mut()
                .find(|item| item.id == criterion)
                .with_context(|| {
                    format!("unknown acceptance criterion {criterion} for task {id}")
                })?;
            if !item.evidence.iter().any(|existing| existing == &evidence) {
                item.evidence.push(evidence.clone());
            }
            item.status = status;
            item.updated_at = now_iso();
            task.updated_at = now_iso();
            append_event(
                project_root,
                "task.acceptance.evidence",
                "verifier",
                Some(&id),
                &format!("{criterion}: {evidence}"),
            )?;
            println!("{}", serde_json::to_string_pretty(item)?);
            Ok(())
        }),
        RuntimeCommand::TaskComplete { id, actor } => {
            validate_id(&actor)?;
            let owner = with_locked_ledger(project_root, |ledger| {
                let blockers = completion_blockers(ledger, &id)?;
                let check = CompletionCheck {
                    task: id.clone(),
                    ready: blockers.is_empty(),
                    blockers,
                };
                if !check.ready {
                    println!("{}", serde_json::to_string_pretty(&check)?);
                    bail!("task {id} is not ready for completion");
                }
                let owner = {
                    let task = ledger
                        .tasks
                        .get_mut(&id)
                        .with_context(|| format!("unknown task: {id}"))?;
                    let owner = task.assignee.take();
                    task.status = "done".to_string();
                    task.waiting_on = None;
                    task.updated_at = now_iso();
                    println!("{}", serde_json::to_string_pretty(task)?);
                    owner
                };
                if let Some(owner_name) = owner.as_deref()
                    && let Some(agent) = ledger.agents.get_mut(owner_name)
                    && agent.task.as_deref() == Some(id.as_str())
                {
                    agent.status = "completed".to_string();
                    agent.task = None;
                    agent.updated_at = now_iso();
                }
                append_event(
                    project_root,
                    "task.completed",
                    &actor,
                    Some(&id),
                    "completion gate passed",
                )?;
                Ok(owner)
            })?;
            if let Some(owner) = owner {
                lease::release_task_if_owned(project_root, &id, &owner).with_context(|| {
                    format!("task {id} completed but its ownership lease could not be released")
                })?;
            }
            Ok(())
        }
        RuntimeCommand::TaskWait { id, await_id } => with_locked_ledger(project_root, |ledger| {
            validate_id(&await_id)?;
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            task.status = "blocked_waiting".to_string();
            task.waiting_on = Some(await_id.clone());
            task.updated_at = now_iso();
            append_event(
                project_root,
                "task.waiting",
                "runtime",
                Some(&id),
                &format!("waiting on {await_id}"),
            )?;
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskWake { id } => with_locked_ledger(project_root, |ledger| {
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            if task.status != "blocked_waiting" {
                bail!("task {id} is not blocked_waiting");
            }
            let await_id = task
                .waiting_on
                .take()
                .context("blocked_waiting task has no await id")?;
            task.status = "doing".to_string();
            task.updated_at = now_iso();
            append_event(
                project_root,
                "task.woke",
                "runtime",
                Some(&id),
                &format!("await {await_id} fired"),
            )?;
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskList => {
            let ledger = load_or_init(project_root)?;
            println!("{}", serde_json::to_string_pretty(&ledger.tasks)?);
            Ok(())
        }
        RuntimeCommand::AgentSet {
            name,
            role,
            status,
            task,
            budget_tokens,
        } => with_locked_ledger(project_root, |ledger| {
            validate_id(&name)?;
            validate_agent_status(&status)?;
            if let Some(task_id) = task.as_deref()
                && !ledger.tasks.contains_key(task_id)
            {
                bail!("unknown task: {task_id}");
            }

            let current = ledger.agents.get(&name).cloned();
            let previous_task = current.as_ref().and_then(|record| record.task.clone());
            let holds_lease = agent_status_holds_task_lease(&status) && task.is_some();
            let new_task_owned = task.clone();
            let new_task = new_task_owned.as_deref();

            if holds_lease && let Some(task_id) = new_task {
                lease::acquire_task(
                    project_root,
                    task_id,
                    &name,
                    lease::DEFAULT_TASK_LEASE_TTL_SECONDS,
                )?;
            }

            if let Some(previous_task_id) = previous_task.as_deref()
                && (Some(previous_task_id) != new_task || !holds_lease)
                && let Err(error) =
                    lease::release_task_if_owned(project_root, previous_task_id, &name)
            {
                if holds_lease
                    && let Some(task_id) = new_task
                    && task_id != previous_task_id
                {
                    let _ = lease::release_task_if_owned(project_root, task_id, &name);
                }
                return Err(error).with_context(|| {
                    format!("release previous task ownership {previous_task_id} for {name}")
                });
            }

            if let Some(previous_task_id) = previous_task.as_deref()
                && (Some(previous_task_id) != new_task || !holds_lease)
                && let Some(record) = ledger.tasks.get_mut(previous_task_id)
                && record.assignee.as_deref() == Some(name.as_str())
            {
                record.assignee = None;
                record.updated_at = now_iso();
            }

            if holds_lease && let Some(task_id) = new_task {
                let record = ledger.tasks.get_mut(task_id).expect("task validated above");
                record.assignee = Some(name.clone());
                record.updated_at = now_iso();
            } else if let Some(task_id) = new_task
                && let Some(record) = ledger.tasks.get_mut(task_id)
                && record.assignee.as_deref() == Some(name.as_str())
            {
                record.assignee = None;
                record.updated_at = now_iso();
            }

            let now = now_iso();
            ledger.agents.insert(
                name.clone(),
                AgentRecord {
                    role,
                    status,
                    task,
                    budget_tokens: budget_tokens.or(current.as_ref().and_then(|v| v.budget_tokens)),
                    used_tokens: current.as_ref().map_or(0, |v| v.used_tokens),
                    last_heartbeat_ms: Utc::now().timestamp_millis(),
                    last_progress: current.as_ref().and_then(|v| v.last_progress.clone()),
                    no_progress_count: current.as_ref().map_or(0, |v| v.no_progress_count),
                    last_action: current.as_ref().and_then(|v| v.last_action.clone()),
                    repeated_action_count: current.as_ref().map_or(0, |v| v.repeated_action_count),
                    consecutive_errors: current.as_ref().map_or(0, |v| v.consecutive_errors),
                    updated_at: now,
                },
            );
            append_event(
                project_root,
                "agent.updated",
                &name,
                new_task,
                if holds_lease {
                    "agent state updated with task ownership lease"
                } else {
                    "agent state updated without task ownership"
                },
            )?;
            println!("{name}");
            Ok(())
        }),
        RuntimeCommand::AgentHeartbeat { name, progress } => {
            with_locked_ledger(project_root, |ledger| {
                let (task_id, status) = {
                    let agent = ledger
                        .agents
                        .get(&name)
                        .with_context(|| format!("unknown agent: {name}"))?;
                    (agent.task.clone(), agent.status.clone())
                };
                if agent_status_holds_task_lease(&status)
                    && let Some(task_id) = task_id.as_deref()
                {
                    lease::renew_task(
                        project_root,
                        task_id,
                        &name,
                        lease::DEFAULT_TASK_LEASE_TTL_SECONDS,
                    )?;
                }
                let agent = ledger.agents.get_mut(&name).expect("agent validated above");
                if progress.is_some() && progress == agent.last_progress {
                    agent.no_progress_count = agent.no_progress_count.saturating_add(1);
                } else if progress.is_some() {
                    agent.no_progress_count = 0;
                }
                if progress.is_some() {
                    agent.last_progress = progress;
                }
                agent.last_heartbeat_ms = Utc::now().timestamp_millis();
                agent.updated_at = now_iso();
                println!("{}", serde_json::to_string_pretty(agent)?);
                Ok(())
            })
        }
        RuntimeCommand::AgentAction {
            name,
            action,
            error,
        } => with_locked_ledger(project_root, |ledger| {
            let agent = ledger
                .agents
                .get_mut(&name)
                .with_context(|| format!("unknown agent: {name}"))?;
            if agent.last_action.as_deref() == Some(action.as_str()) {
                agent.repeated_action_count = agent.repeated_action_count.saturating_add(1);
            } else {
                agent.last_action = Some(action);
                agent.repeated_action_count = 1;
            }
            if error {
                agent.consecutive_errors = agent.consecutive_errors.saturating_add(1);
            } else {
                agent.consecutive_errors = 0;
            }
            agent.updated_at = now_iso();
            println!("{}", serde_json::to_string_pretty(agent)?);
            Ok(())
        }),
        RuntimeCommand::AgentTokens { name, add } => with_locked_ledger(project_root, |ledger| {
            let (task_id, status) = {
                let agent = ledger
                    .agents
                    .get(&name)
                    .with_context(|| format!("unknown agent: {name}"))?;
                (agent.task.clone(), agent.status.clone())
            };
            if agent_status_holds_task_lease(&status)
                && let Some(task_id) = task_id.as_deref()
            {
                lease::renew_task(
                    project_root,
                    task_id,
                    &name,
                    lease::DEFAULT_TASK_LEASE_TTL_SECONDS,
                )?;
            }
            let agent = ledger.agents.get_mut(&name).expect("agent validated above");
            agent.used_tokens = agent.used_tokens.saturating_add(add);
            if let Some(task_id) = task_id
                && let Some(task) = ledger.tasks.get_mut(&task_id)
            {
                task.used_tokens = task.used_tokens.saturating_add(add);
                task.updated_at = now_iso();
            }
            agent.updated_at = now_iso();
            println!("{}", serde_json::to_string_pretty(agent)?);
            Ok(())
        }),
        RuntimeCommand::AgentList => {
            let ledger = load_or_init(project_root)?;
            println!("{}", serde_json::to_string_pretty(&ledger.agents)?);
            Ok(())
        }
        RuntimeCommand::GateSet {
            task,
            name,
            status,
            risk,
            reviewer,
            finding,
        } => with_locked_ledger(project_root, |ledger| {
            validate_gate_status(&status)?;
            validate_risk(&risk)?;
            let task_record = ledger
                .tasks
                .get_mut(&task)
                .with_context(|| format!("unknown task: {task}"))?;
            task_record.gates.insert(
                name.clone(),
                GateRecord {
                    status,
                    risk,
                    reviewer: reviewer.clone(),
                    finding,
                    updated_at: now_iso(),
                },
            );
            task_record.updated_at = now_iso();
            append_event(
                project_root,
                "gate.updated",
                reviewer.as_deref().unwrap_or("god"),
                Some(&task),
                &format!("gate {name} updated"),
            )?;
            Ok(())
        }),
        RuntimeCommand::HandoffAdd {
            task,
            from_actor,
            to_actor,
            summary,
            refs,
            accomplished,
            remaining_work,
            failures,
            relevant_files,
            decisions,
            rationale,
            restart_commands,
            next_action,
        } => with_locked_ledger(project_root, |ledger| {
            validate_id(&from_actor)?;
            validate_id(&to_actor)?;
            validate_handoff_text(&summary, "handoff summary")?;
            validate_handoff_items(&refs, "handoff ref")?;
            validate_handoff_items(&accomplished, "handoff accomplished item")?;
            validate_handoff_items(&remaining_work, "handoff remaining item")?;
            validate_handoff_items(&failures, "handoff failure item")?;
            validate_handoff_items(&relevant_files, "handoff file")?;
            validate_handoff_items(&decisions, "handoff decision")?;
            validate_handoff_items(&restart_commands, "handoff restart command")?;
            if let Some(value) = rationale.as_deref() {
                validate_handoff_text(value, "handoff rationale")?;
            }
            if let Some(value) = next_action.as_deref() {
                validate_handoff_text(value, "handoff next action")?;
            }
            let task_record = ledger
                .tasks
                .get_mut(&task)
                .with_context(|| format!("unknown task: {task}"))?;
            task_record.handoffs.push(HandoffRecord {
                from: from_actor.clone(),
                to: to_actor.clone(),
                summary,
                refs,
                accomplished,
                remaining_work,
                failures,
                relevant_files,
                decisions,
                rationale,
                restart_commands,
                next_action,
                at: now_iso(),
            });
            task_record.updated_at = now_iso();
            append_event(
                project_root,
                "handoff",
                &from_actor,
                Some(&task),
                &format!("handoff to {to_actor}"),
            )?;
            Ok(())
        }),
        RuntimeCommand::Snapshot => {
            let ledger = load_or_init(project_root)?;
            println!("{}", serde_json::to_string_pretty(&ledger)?);
            Ok(())
        }
        RuntimeCommand::Supervise {
            apply,
            stale_seconds,
            max_repeated_action,
            max_consecutive_errors,
            max_no_progress,
            max_live_agents,
        } => supervise(
            project_root,
            apply,
            stale_seconds,
            max_repeated_action,
            max_consecutive_errors,
            max_no_progress,
            max_live_agents,
        ),
    }
}

pub fn seed_orchestration_plan(
    project_root: &Path,
    task_id: &str,
    title: &str,
    risk: &str,
    gates: &[String],
) -> Result<()> {
    validate_id(task_id)?;
    validate_risk(risk)?;
    with_locked_ledger(project_root, |ledger| {
        let now = now_iso();
        let task = ledger
            .tasks
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRecord {
                title: title.to_string(),
                status: "todo".to_string(),
                risk: risk.to_string(),
                assignee: None,
                depends_on: Vec::new(),
                budget_tokens: None,
                used_tokens: 0,
                waiting_on: None,
                acceptance: Vec::new(),
                gates: BTreeMap::new(),
                handoffs: Vec::new(),
                updated_at: now.clone(),
            });
        task.title = title.to_string();
        task.risk = risk.to_string();
        task.updated_at = now.clone();
        for gate in gates {
            task.gates
                .entry(gate.clone())
                .or_insert_with(|| GateRecord {
                    status: "block".to_string(),
                    risk: risk.to_string(),
                    reviewer: None,
                    finding: Some("pending orchestration gate".to_string()),
                    updated_at: now.clone(),
                });
        }
        append_event(
            project_root,
            "orchestration.plan",
            "god",
            Some(task_id),
            "task and pending gates seeded",
        )?;
        Ok(())
    })
}

fn completion_blockers(ledger: &RuntimeLedger, task_id: &str) -> Result<Vec<String>> {
    let task = ledger
        .tasks
        .get(task_id)
        .with_context(|| format!("unknown task: {task_id}"))?;
    let mut blockers = Vec::new();

    if task.acceptance.is_empty() {
        blockers.push("no acceptance criteria are recorded".to_string());
    }
    for criterion in &task.acceptance {
        if criterion.status != "pass" {
            blockers.push(format!(
                "acceptance {} is {}: {}",
                criterion.id, criterion.status, criterion.text
            ));
        } else if criterion.evidence.is_empty() {
            blockers.push(format!("acceptance {} has no evidence", criterion.id));
        }
    }
    for dependency in &task.depends_on {
        match ledger.tasks.get(dependency) {
            Some(record) if record.status == "done" => {}
            Some(record) => blockers.push(format!(
                "dependency {dependency} is {} instead of done",
                record.status
            )),
            None => blockers.push(format!("dependency {dependency} does not exist")),
        }
    }
    for (name, gate) in &task.gates {
        if gate.status == "block" {
            blockers.push(format!("gate {name} is blocking"));
        }
    }
    if task.status == "cancelled" || task.status == "failed" {
        blockers.push(format!("task status {} cannot complete", task.status));
    }
    Ok(blockers)
}

fn acceptance_records(values: Vec<String>) -> Result<Vec<AcceptanceCriterion>> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            validate_acceptance_text(&text)?;
            Ok(AcceptanceCriterion {
                id: format!("ac-{}", index + 1),
                text,
                status: "pending".to_string(),
                evidence: Vec::new(),
                updated_at: now_iso(),
            })
        })
        .collect()
}

fn supervise(
    project_root: &Path,
    apply: bool,
    stale_seconds: u64,
    max_repeated_action: u32,
    max_consecutive_errors: u32,
    max_no_progress: u32,
    max_live_agents: usize,
) -> Result<()> {
    with_locked_ledger(project_root, |ledger| {
        let now_ms = Utc::now().timestamp_millis();
        let stale_ms = i64::try_from(stale_seconds)
            .unwrap_or(i64::MAX / 1000)
            .saturating_mul(1000);
        let live_count = ledger
            .agents
            .values()
            .filter(|agent| LIVE_STATUSES.contains(&agent.status.as_str()))
            .count();
        let mut findings = Vec::new();
        if live_count > max_live_agents {
            findings.push(BreakerFinding {
                agent: None,
                kind: "live_agent_budget".to_string(),
                detail: format!("{live_count} live agents exceeds limit {max_live_agents}"),
            });
        }
        for (name, agent) in &mut ledger.agents {
            if !LIVE_STATUSES.contains(&agent.status.as_str()) {
                continue;
            }
            let mut reasons = Vec::new();
            if now_ms.saturating_sub(agent.last_heartbeat_ms) > stale_ms {
                reasons.push("heartbeat stale".to_string());
            }
            if agent.repeated_action_count >= max_repeated_action {
                reasons.push(format!(
                    "repeated action count {}",
                    agent.repeated_action_count
                ));
            }
            if agent.consecutive_errors >= max_consecutive_errors {
                reasons.push(format!("consecutive errors {}", agent.consecutive_errors));
            }
            if agent.no_progress_count >= max_no_progress {
                reasons.push(format!("no-progress count {}", agent.no_progress_count));
            }
            if let Some(limit) = agent.budget_tokens
                && agent.used_tokens >= limit
            {
                reasons.push(format!(
                    "agent token budget exhausted {}/{}",
                    agent.used_tokens, limit
                ));
            }
            if let Some(task_id) = agent.task.as_deref()
                && let Some(task) = ledger.tasks.get(task_id)
                && let Some(limit) = task.budget_tokens
                && task.used_tokens >= limit
            {
                reasons.push(format!(
                    "task token budget exhausted {}/{}",
                    task.used_tokens, limit
                ));
            }
            if !reasons.is_empty() {
                findings.push(BreakerFinding {
                    agent: Some(name.clone()),
                    kind: "agent_breaker".to_string(),
                    detail: reasons.join("; "),
                });
                if apply {
                    agent.status = "blocked".to_string();
                    agent.updated_at = now_iso();
                }
            }
        }
        if apply {
            for finding in &findings {
                append_event(
                    project_root,
                    "breaker",
                    finding.agent.as_deref().unwrap_or("runtime"),
                    None,
                    &finding.detail,
                )?;
            }
        }
        println!("{}", serde_json::to_string_pretty(&findings)?);
        Ok(())
    })
}

fn runtime_dir(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("state")
}

fn ledger_path(project_root: &Path) -> PathBuf {
    runtime_dir(project_root).join("runtime-v2.json")
}

fn event_path(project_root: &Path) -> PathBuf {
    runtime_dir(project_root).join("events-v2.jsonl")
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn default_ledger(project_root: &Path) -> RuntimeLedger {
    let now = now_iso();
    RuntimeLedger {
        schema: SCHEMA.to_string(),
        project_root: project_root.display().to_string(),
        created_at: now.clone(),
        updated_at: now,
        tasks: BTreeMap::new(),
        agents: BTreeMap::new(),
    }
}

fn load_or_init(project_root: &Path) -> Result<RuntimeLedger> {
    fs::create_dir_all(runtime_dir(project_root)).context("create CodexFlow runtime directory")?;
    let path = ledger_path(project_root);
    if !path.exists() {
        let ledger = default_ledger(project_root);
        save_ledger(project_root, &ledger)?;
        return Ok(ledger);
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut ledger: RuntimeLedger =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    if ledger.schema != SCHEMA {
        bail!("unsupported runtime schema {}", ledger.schema);
    }
    ledger.project_root = project_root.display().to_string();
    Ok(ledger)
}

fn save_ledger(project_root: &Path, ledger: &RuntimeLedger) -> Result<()> {
    let path = ledger_path(project_root);
    fs::create_dir_all(path.parent().context("ledger parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut next = ledger.clone();
    next.updated_at = now_iso();
    fs::write(&tmp, serde_json::to_vec_pretty(&next)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    if cfg!(windows) && path.exists() {
        fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

fn append_event(
    project_root: &Path,
    kind: &str,
    actor: &str,
    task: Option<&str>,
    message: &str,
) -> Result<()> {
    fs::create_dir_all(runtime_dir(project_root))?;
    let path = event_path(project_root);
    let event = serde_json::json!({
        "ts": now_iso(),
        "kind": kind,
        "actor": actor,
        "task": task,
        "message": message
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&event)?)?;
    Ok(())
}

fn with_locked_ledger<T>(
    project_root: &Path,
    f: impl FnOnce(&mut RuntimeLedger) -> Result<T>,
) -> Result<T> {
    let _guard = StateLock::acquire(project_root)?;
    let mut ledger = load_or_init(project_root)?;
    let output = f(&mut ledger)?;
    save_ledger(project_root, &ledger)?;
    Ok(output)
}

struct StateLock {
    path: PathBuf,
}

impl StateLock {
    fn acquire(project_root: &Path) -> Result<Self> {
        fs::create_dir_all(runtime_dir(project_root))?;
        let path = runtime_dir(project_root).join(".runtime.lock");
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "pid={}", std::process::id())?;
                    return Ok(Self { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|elapsed| elapsed > Duration::from_secs(120));
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > Duration::from_secs(8) {
                        bail!("timed out waiting for runtime lock {}", path.display());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(err).context("create runtime lock"),
            }
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        bail!("invalid id {value:?}; use lowercase letters, digits, _ or -, max 64 chars");
    }
    Ok(())
}

fn validate_risk(value: &str) -> Result<()> {
    if !["low", "medium", "high", "critical"].contains(&value) {
        bail!("invalid risk {value}");
    }
    Ok(())
}

fn validate_task_status(value: &str) -> Result<()> {
    if ![
        "todo",
        "doing",
        "blocked",
        "blocked_waiting",
        "review",
        "done",
        "failed",
        "cancelled",
    ]
    .contains(&value)
    {
        bail!("invalid task status {value}");
    }
    Ok(())
}

fn validate_acceptance_status(value: &str) -> Result<()> {
    if !["pending", "pass", "fail"].contains(&value) {
        bail!("invalid acceptance status {value}");
    }
    Ok(())
}

fn validate_acceptance_text(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 1024 {
        bail!("acceptance criterion must be 1 to 1024 characters");
    }
    Ok(())
}

fn validate_evidence(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 2048 {
        bail!("evidence must be 1 to 2048 characters");
    }
    Ok(())
}

fn agent_status_holds_task_lease(value: &str) -> bool {
    matches!(value, "pending" | "running" | "idle")
}

fn validate_handoff_text(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 2048 {
        bail!("{label} must be 1 to 2048 characters");
    }
    Ok(())
}

fn validate_handoff_items(values: &[String], label: &str) -> Result<()> {
    if values.len() > 128 {
        bail!("{label} list cannot exceed 128 items");
    }
    for value in values {
        validate_handoff_text(value, label)?;
    }
    Ok(())
}

fn validate_agent_status(value: &str) -> Result<()> {
    if ![
        "pending",
        "running",
        "idle",
        "blocked",
        "completed",
        "failed",
        "closed",
    ]
    .contains(&value)
    {
        bail!("invalid agent status {value}");
    }
    Ok(())
}

fn validate_gate_status(value: &str) -> Result<()> {
    if !["pass", "warn", "block", "not_applicable"].contains(&value) {
        bail!("invalid gate status {value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_with_acceptance(text: &str) -> TaskRecord {
        TaskRecord {
            title: "demo".to_string(),
            status: "review".to_string(),
            risk: "medium".to_string(),
            assignee: None,
            depends_on: Vec::new(),
            budget_tokens: None,
            used_tokens: 0,
            waiting_on: None,
            acceptance: vec![AcceptanceCriterion {
                id: "ac-1".to_string(),
                text: text.to_string(),
                status: "pending".to_string(),
                evidence: Vec::new(),
                updated_at: now_iso(),
            }],
            gates: BTreeMap::new(),
            handoffs: Vec::new(),
            updated_at: now_iso(),
        }
    }

    #[test]
    fn legacy_handoff_deserializes_with_structured_defaults() {
        let handoff: HandoffRecord = serde_json::from_value(serde_json::json!({
            "from": "worker-a",
            "to": "worker-b",
            "summary": "legacy",
            "refs": ["src/lib.rs"],
            "at": "2026-09-05T00:00:00Z"
        }))
        .expect("legacy handoff");
        assert!(handoff.accomplished.is_empty());
        assert!(handoff.remaining_work.is_empty());
        assert!(handoff.restart_commands.is_empty());
        assert!(handoff.next_action.is_none());
    }

    #[test]
    fn completion_requires_passing_evidence() {
        let mut ledger = default_ledger(Path::new("/demo"));
        ledger
            .tasks
            .insert("task-a".to_string(), task_with_acceptance("feature works"));
        let blockers = completion_blockers(&ledger, "task-a").expect("check blockers");
        assert_eq!(blockers.len(), 1);
        assert!(blockers[0].contains("acceptance ac-1 is pending"));

        let task = ledger.tasks.get_mut("task-a").expect("task");
        task.acceptance[0].status = "pass".to_string();
        task.acceptance[0]
            .evidence
            .push("cargo test: pass".to_string());
        assert!(
            completion_blockers(&ledger, "task-a")
                .expect("check ready")
                .is_empty()
        );
    }

    #[test]
    fn completion_requires_done_dependencies_and_non_blocking_gates() {
        let mut ledger = default_ledger(Path::new("/demo"));
        let mut task = task_with_acceptance("integration passes");
        task.acceptance[0].status = "pass".to_string();
        task.acceptance[0]
            .evidence
            .push("integration suite".to_string());
        task.depends_on.push("dep".to_string());
        task.gates.insert(
            "security".to_string(),
            GateRecord {
                status: "block".to_string(),
                risk: "high".to_string(),
                reviewer: Some("reviewer".to_string()),
                finding: Some("open finding".to_string()),
                updated_at: now_iso(),
            },
        );
        ledger.tasks.insert("task-a".to_string(), task);
        ledger.tasks.insert(
            "dep".to_string(),
            TaskRecord {
                title: "dependency".to_string(),
                status: "doing".to_string(),
                risk: "low".to_string(),
                assignee: None,
                depends_on: Vec::new(),
                budget_tokens: None,
                used_tokens: 0,
                waiting_on: None,
                acceptance: Vec::new(),
                gates: BTreeMap::new(),
                handoffs: Vec::new(),
                updated_at: now_iso(),
            },
        );
        let blockers = completion_blockers(&ledger, "task-a").expect("check blockers");
        assert!(blockers.iter().any(|item| item.contains("dependency dep")));
        assert!(blockers.iter().any(|item| item.contains("gate security")));
    }
}
