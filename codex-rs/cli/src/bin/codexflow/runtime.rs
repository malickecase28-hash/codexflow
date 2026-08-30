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
    gates: BTreeMap<String, GateRecord>,
    handoffs: Vec<HandoffRecord>,
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
    at: String,
}

#[derive(Debug, Clone, Serialize)]
struct BreakerFinding {
    agent: Option<String>,
    kind: String,
    detail: String,
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
        } => with_locked_ledger(project_root, |ledger| {
            validate_id(&id)?;
            validate_risk(&risk)?;
            if ledger.tasks.contains_key(&id) {
                bail!("task already exists: {id}");
            }
            let now = now_iso();
            ledger.tasks.insert(
                id.clone(),
                TaskRecord {
                    title,
                    status: "todo".to_string(),
                    risk,
                    assignee,
                    depends_on,
                    budget_tokens: None,
                    used_tokens: 0,
                    waiting_on: None,
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
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            if let Some(status) = status {
                validate_task_status(&status)?;
                if status == "blocked_waiting" {
                    bail!("use task-wait so blocked_waiting records the await id");
                }
                task.status = status;
                task.waiting_on = None;
            }
            if let Some(risk) = risk {
                validate_risk(&risk)?;
                task.risk = risk;
            }
            if assignee.is_some() {
                task.assignee = assignee;
            }
            if budget_tokens.is_some() {
                task.budget_tokens = budget_tokens;
            }
            task.updated_at = now_iso();
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskWait { id, await_id } => {
            with_locked_ledger(project_root, |ledger| {
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
            })
        }
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
            let now = now_iso();
            let current = ledger.agents.get(&name).cloned();
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
                None,
                "agent state updated",
            )?;
            println!("{name}");
            Ok(())
        }),
        RuntimeCommand::AgentHeartbeat { name, progress } => {
            with_locked_ledger(project_root, |ledger| {
                let agent = ledger
                    .agents
                    .get_mut(&name)
                    .with_context(|| format!("unknown agent: {name}"))?;
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
        RuntimeCommand::AgentTokens { name, add } => {
            with_locked_ledger(project_root, |ledger| {
                let agent = ledger
                    .agents
                    .get_mut(&name)
                    .with_context(|| format!("unknown agent: {name}"))?;
                agent.used_tokens = agent.used_tokens.saturating_add(add);
                if let Some(task_id) = agent.task.clone()
                    && let Some(task) = ledger.tasks.get_mut(&task_id)
                {
                    task.used_tokens = task.used_tokens.saturating_add(add);
                    task.updated_at = now_iso();
                }
                agent.updated_at = now_iso();
                println!("{}", serde_json::to_string_pretty(agent)?);
                Ok(())
            })
        }
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
        } => with_locked_ledger(project_root, |ledger| {
            let task_record = ledger
                .tasks
                .get_mut(&task)
                .with_context(|| format!("unknown task: {task}"))?;
            task_record.handoffs.push(HandoffRecord {
                from: from_actor.clone(),
                to: to_actor.clone(),
                summary,
                refs,
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
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'_'
                || byte == b'-'
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
