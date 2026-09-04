use crate::runtime_state;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::SecondsFormat;
use chrono::Utc;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const LEASE_SCHEMA: &str = "codexflow.lease.v1";
const MIN_LEASE_TTL_SECONDS: u64 = 30;
const MAX_LEASE_TTL_SECONDS: u64 = 86_400;

#[derive(Debug, Args)]
pub struct OrchestrateArgs {
    /// Project id or unique name. Omit to resolve from the current directory.
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: OrchestrateCommand,
}

#[derive(Debug, Subcommand)]
enum OrchestrateCommand {
    /// Create a project orchestration manifest.
    Init {
        #[arg(long, default_value = "engineering")]
        preset: String,
        #[arg(long)]
        force: bool,
    },
    /// Show the effective orchestration configuration.
    Show,
    /// Create an execution plan for a task.
    Plan {
        #[arg(long)]
        task: String,
        #[arg(long)]
        risk: Option<String>,
        #[arg(long)]
        task_id: Option<String>,
        /// Seed the runtime task and blocking gates.
        #[arg(long)]
        apply: bool,
    },
    /// List skill names visible to the deterministic capability resolver.
    Skills,
    /// Manage atomic TTL ownership leases shared across Git worktrees.
    Lease {
        #[command(subcommand)]
        command: LeaseCommand,
    },
}

#[derive(Debug, Subcommand)]
enum LeaseCommand {
    Acquire {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long, default_value_t = 900)]
        ttl_seconds: u64,
    },
    Renew {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        owner: String,
        #[arg(long, default_value_t = 900)]
        ttl_seconds: u64,
    },
    Release {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        owner: String,
    },
    List,
    Prune,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct OrchestrationConfig {
    schema: String,
    independent_review: bool,
    reviewer_role: String,
    default_worker_role: String,
    departments: Vec<Department>,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        engineering_preset()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Department {
    id: String,
    description: String,
    triggers: Vec<String>,
    skills: Vec<String>,
    role: String,
    model: Option<String>,
    blocking_risks: Vec<String>,
}

impl Default for Department {
    fn default() -> Self {
        Self {
            id: String::new(),
            description: String::new(),
            triggers: Vec::new(),
            skills: Vec::new(),
            role: "flow_reviewer".to_string(),
            model: None,
            blocking_risks: vec!["high".to_string(), "critical".to_string()],
        }
    }
}

#[derive(Debug, Serialize)]
struct ExecutionPlan {
    schema: &'static str,
    task: String,
    task_id: Option<String>,
    lease_scope: Option<String>,
    risk: String,
    topology: String,
    selected_departments: Vec<SelectedDepartment>,
    selected_skills: Vec<String>,
    missing_skills: Vec<String>,
    roles: Vec<String>,
    blocking_gates: Vec<String>,
    independent_review: bool,
    reviewer_role: String,
}

#[derive(Debug, Serialize)]
struct SelectedDepartment {
    id: String,
    score: usize,
    role: String,
    model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LeaseRecord {
    schema: String,
    scope: String,
    owner: String,
    task: Option<String>,
    acquired_at: String,
    renewed_at: String,
    expires_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct LeaseView {
    #[serde(flatten)]
    lease: LeaseRecord,
    expired: bool,
}

pub fn handle(project_root: &Path, args: OrchestrateArgs) -> Result<()> {
    match args.command {
        OrchestrateCommand::Init { preset, force } => init(project_root, &preset, force),
        OrchestrateCommand::Show => {
            println!(
                "{}",
                serde_json::to_string_pretty(&load_config(project_root)?)?
            );
            Ok(())
        }
        OrchestrateCommand::Plan {
            task,
            risk,
            task_id,
            apply,
        } => {
            let plan = plan(project_root, task, risk, task_id.clone())?;
            if apply {
                let id = task_id.context("--apply requires --task-id")?;
                runtime_state::seed_orchestration_plan(
                    project_root,
                    &id,
                    &plan.task,
                    &plan.risk,
                    &plan.blocking_gates,
                )?;
            }
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        OrchestrateCommand::Skills => {
            let skills = discover_skill_names(project_root)?;
            println!("{}", serde_json::to_string_pretty(&skills)?);
            Ok(())
        }
        OrchestrateCommand::Lease { command } => handle_lease(project_root, command),
    }
}

fn handle_lease(project_root: &Path, command: LeaseCommand) -> Result<()> {
    match command {
        LeaseCommand::Acquire {
            scope,
            owner,
            task,
            ttl_seconds,
        } => {
            let lease = acquire_lease(project_root, &scope, &owner, task.as_deref(), ttl_seconds)?;
            println!("{}", serde_json::to_string_pretty(&lease)?);
            Ok(())
        }
        LeaseCommand::Renew {
            scope,
            owner,
            ttl_seconds,
        } => {
            let lease = renew_lease(project_root, &scope, &owner, ttl_seconds)?;
            println!("{}", serde_json::to_string_pretty(&lease)?);
            Ok(())
        }
        LeaseCommand::Release { scope, owner } => {
            release_lease(project_root, &scope, &owner)?;
            println!("{scope}");
            Ok(())
        }
        LeaseCommand::List => {
            println!(
                "{}",
                serde_json::to_string_pretty(&list_leases(project_root)?)?
            );
            Ok(())
        }
        LeaseCommand::Prune => {
            println!(
                "{}",
                serde_json::to_string_pretty(&prune_expired_leases(project_root)?)?
            );
            Ok(())
        }
    }
}

fn init(project_root: &Path, preset: &str, force: bool) -> Result<()> {
    let config = match preset {
        "engineering" => engineering_preset(),
        "minimal" => OrchestrationConfig {
            departments: Vec::new(),
            ..OrchestrationConfig::default()
        },
        _ => bail!("unknown preset {preset}; expected engineering or minimal"),
    };
    let path = config_path(project_root);
    if path.exists() && !force {
        bail!(
            "orchestration config already exists at {}; use --force to replace it",
            path.display()
        );
    }
    fs::create_dir_all(path.parent().context("orchestration config parent")?)?;
    fs::write(&path, serde_json::to_vec_pretty(&config)?)
        .with_context(|| format!("write {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

fn plan(
    project_root: &Path,
    task: String,
    explicit_risk: Option<String>,
    task_id: Option<String>,
) -> Result<ExecutionPlan> {
    if let Some(id) = task_id.as_deref() {
        validate_lease_token(id, "task id")?;
    }
    let lease_scope = task_id.as_ref().map(|id| format!("task-{id}"));
    let config = load_config(project_root)?;
    let task_lower = task.to_ascii_lowercase();
    let risk = explicit_risk.unwrap_or_else(|| classify_risk(&task_lower));
    validate_risk(&risk)?;

    let available_skills = discover_skill_names(project_root)?;
    let available_set = available_skills.iter().cloned().collect::<BTreeSet<_>>();
    let mut selected = Vec::new();

    for department in &config.departments {
        let score = department
            .triggers
            .iter()
            .filter(|trigger| task_lower.contains(&trigger.to_ascii_lowercase()))
            .count();
        if score > 0 {
            selected.push(SelectedDepartment {
                id: department.id.clone(),
                score,
                role: department.role.clone(),
                model: department.model.clone(),
            });
        }
    }
    selected.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });

    let selected_ids = selected
        .iter()
        .map(|department| department.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut selected_skills = BTreeSet::new();
    let mut missing_skills = BTreeSet::new();
    let mut roles = BTreeSet::new();
    roles.insert(config.default_worker_role.clone());

    let mut gates = BTreeSet::new();
    for department in &config.departments {
        if !selected_ids.contains(department.id.as_str()) {
            continue;
        }
        roles.insert(department.role.clone());
        for skill in &department.skills {
            if available_set.contains(skill) {
                selected_skills.insert(skill.clone());
            } else {
                missing_skills.insert(skill.clone());
            }
        }
        if department.blocking_risks.iter().any(|item| item == &risk) {
            gates.insert(department.id.clone());
        }
    }

    let independent_review = config.independent_review && risk != "low";
    if independent_review {
        roles.insert(config.reviewer_role.clone());
        gates.insert("independent_review".to_string());
    }

    let topology = topology_for(&risk, selected.len(), &task_lower).to_string();

    Ok(ExecutionPlan {
        schema: "codexflow.plan.v1",
        task,
        task_id,
        lease_scope,
        risk,
        topology,
        selected_departments: selected,
        selected_skills: selected_skills.into_iter().collect(),
        missing_skills: missing_skills.into_iter().collect(),
        roles: roles.into_iter().collect(),
        blocking_gates: gates.into_iter().collect(),
        independent_review,
        reviewer_role: config.reviewer_role,
    })
}

fn acquire_lease(
    project_root: &Path,
    scope: &str,
    owner: &str,
    task: Option<&str>,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    validate_lease_token(scope, "lease scope")?;
    validate_lease_token(owner, "lease owner")?;
    if let Some(task) = task {
        validate_lease_token(task, "task id")?;
    }
    let ttl_seconds = validate_lease_ttl(ttl_seconds)?;
    let dir = lease_dir(project_root)?;
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = lease_path(&dir, scope);

    for _ in 0..4 {
        let now = Utc::now();
        let lease = LeaseRecord {
            schema: LEASE_SCHEMA.to_string(),
            scope: scope.to_string(),
            owner: owner.to_string(),
            task: task.map(str::to_string),
            acquired_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
            renewed_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
            expires_at_ms: now
                .timestamp_millis()
                .saturating_add(ttl_millis(ttl_seconds)),
        };
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&serde_json::to_vec_pretty(&lease)?)?;
                file.sync_all()?;
                return Ok(lease);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = load_lease_path(&path)?;
                if existing.owner == owner && !lease_expired(&existing) {
                    return renew_lease(project_root, scope, owner, ttl_seconds);
                }
                if lease_expired(&existing) {
                    match fs::remove_file(&path) {
                        Ok(()) => continue,
                        Err(remove_err)
                            if remove_err.kind() == std::io::ErrorKind::NotFound =>
                        {
                            continue;
                        }
                        Err(remove_err) => {
                            return Err(remove_err)
                                .with_context(|| format!("remove expired {}", path.display()));
                        }
                    }
                }
                bail!(
                    "lease {scope} is held by {} until {}",
                    existing.owner,
                    existing.expires_at_ms
                );
            }
            Err(err) => return Err(err).with_context(|| format!("create {}", path.display())),
        }
    }
    bail!("lease {scope} changed repeatedly while acquiring; retry the operation")
}

fn renew_lease(
    project_root: &Path,
    scope: &str,
    owner: &str,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    validate_lease_token(scope, "lease scope")?;
    validate_lease_token(owner, "lease owner")?;
    let ttl_seconds = validate_lease_ttl(ttl_seconds)?;
    let dir = lease_dir(project_root)?;
    let path = lease_path(&dir, scope);
    let mut lease = load_lease_path(&path)?;
    if lease.owner != owner {
        bail!("lease {scope} is owned by {}, not {owner}", lease.owner);
    }
    if lease_expired(&lease) {
        bail!("lease {scope} has expired; acquire it again instead of renewing it");
    }
    let now = Utc::now();
    lease.renewed_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    lease.expires_at_ms = now
        .timestamp_millis()
        .saturating_add(ttl_millis(ttl_seconds));
    atomic_write_json(&path, &lease)?;
    Ok(lease)
}

fn release_lease(project_root: &Path, scope: &str, owner: &str) -> Result<()> {
    validate_lease_token(scope, "lease scope")?;
    validate_lease_token(owner, "lease owner")?;
    let dir = lease_dir(project_root)?;
    let path = lease_path(&dir, scope);
    let lease = load_lease_path(&path)?;
    if lease.owner != owner {
        bail!("lease {scope} is owned by {}, not {owner}", lease.owner);
    }
    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))
}

fn list_leases(project_root: &Path) -> Result<Vec<LeaseView>> {
    let dir = lease_dir(project_root)?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut leases = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.path().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let lease = match load_lease_path(&entry.path()) {
            Ok(lease) => lease,
            Err(_) => continue,
        };
        leases.push(LeaseView {
            expired: lease_expired(&lease),
            lease,
        });
    }
    leases.sort_by(|left, right| left.lease.scope.cmp(&right.lease.scope));
    Ok(leases)
}

fn prune_expired_leases(project_root: &Path) -> Result<Vec<String>> {
    let dir = lease_dir(project_root)?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let lease = match load_lease_path(&path) {
            Ok(lease) => lease,
            Err(_) => continue,
        };
        if lease_expired(&lease) {
            match fs::remove_file(&path) {
                Ok(()) => removed.push(lease.scope),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
            }
        }
    }
    removed.sort();
    Ok(removed)
}

fn lease_dir(project_root: &Path) -> Result<PathBuf> {
    let raw = git_output(project_root, &["rev-parse", "--git-common-dir"])?;
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("lease operations require a Git repository");
    }
    let common = PathBuf::from(raw);
    let common = if common.is_absolute() {
        common
    } else {
        project_root.join(common)
    };
    Ok(common.join("codexflow").join("leases"))
}

fn lease_path(dir: &Path, scope: &str) -> PathBuf {
    dir.join(format!("{scope}.json"))
}

fn load_lease_path(path: &Path) -> Result<LeaseRecord> {
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let lease: LeaseRecord =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    if lease.schema != LEASE_SCHEMA {
        bail!("unsupported lease schema {}", lease.schema);
    }
    Ok(lease)
}

fn lease_expired(lease: &LeaseRecord) -> bool {
    Utc::now().timestamp_millis() >= lease.expires_at_ms
}

fn ttl_millis(ttl_seconds: u64) -> i64 {
    i64::try_from(ttl_seconds)
        .unwrap_or(i64::MAX / 1000)
        .saturating_mul(1000)
}

fn validate_lease_ttl(ttl_seconds: u64) -> Result<u64> {
    if !(MIN_LEASE_TTL_SECONDS..=MAX_LEASE_TTL_SECONDS).contains(&ttl_seconds) {
        bail!(
            "lease ttl must be between {MIN_LEASE_TTL_SECONDS} and {MAX_LEASE_TTL_SECONDS} seconds"
        );
    }
    Ok(ttl_seconds)
}

fn validate_lease_token(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        bail!(
            "invalid {label} {value:?}; use lowercase letters, digits, _ or -, max 64 chars"
        );
    }
    Ok(())
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::create_dir_all(path.parent().context("lease parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    if cfg!(windows) && path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))
}

fn git_output(project_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(project_root)
        .args(args)
        .output()
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn config_path(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("orchestration.json")
}

fn load_config(project_root: &Path) -> Result<OrchestrationConfig> {
    let path = config_path(project_root);
    if path.exists() {
        let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        return serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()));
    }

    let legacy = project_root
        .join("docs")
        .join("maintenance")
        .join("departments.json");
    if legacy.exists() {
        let data =
            fs::read_to_string(&legacy).with_context(|| format!("read {}", legacy.display()))?;
        let value: serde_json::Value =
            serde_json::from_str(&data).with_context(|| format!("parse {}", legacy.display()))?;
        if let Some(items) = value.get("departments").and_then(|value| value.as_array()) {
            let mut config = engineering_preset();
            for item in items {
                let Some(id) = item.get("id").and_then(|value| value.as_str()) else {
                    continue;
                };
                let Some(department) = config.departments.iter_mut().find(|entry| entry.id == id)
                else {
                    continue;
                };
                if let Some(skills) = item.get("skills").and_then(|value| value.as_array()) {
                    department.skills = skills
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect();
                }
            }
            return Ok(config);
        }
    }

    Ok(engineering_preset())
}

fn discover_skill_names(project_root: &Path) -> Result<Vec<String>> {
    let mut roots = vec![
        project_root.join(".codex").join("skills"),
        project_root.join(".claude").join("skills"),
        project_root.join("skills"),
    ];
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|path| path.join(".codex")));
    if let Some(home) = codex_home {
        roots.push(home.join("skills"));
    }

    let mut names = BTreeSet::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        collect_skills(&root, 0, &mut names)?;
    }
    Ok(names.into_iter().collect())
}

fn collect_skills(root: &Path, depth: usize, names: &mut BTreeSet<String>) -> Result<()> {
    if depth > 3 {
        return Ok(());
    }
    for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_skills(&path, depth + 1, names)?;
        } else if entry.file_name().to_string_lossy() == "SKILL.md" {
            let text = fs::read_to_string(&path).unwrap_or_default();
            if let Some(name) = text
                .lines()
                .find_map(|line| line.strip_prefix("name:").map(str::trim))
                .filter(|name| !name.is_empty())
            {
                names.insert(name.trim_matches('"').to_string());
            } else if let Some(parent) = path.parent().and_then(Path::file_name) {
                names.insert(parent.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

fn classify_risk(task: &str) -> String {
    if contains_any(
        task,
        &[
            "production",
            "live trading",
            "live order",
            "credential",
            "secret",
            "deploy",
            "migration",
            "payment",
            "authentication",
            "authorization",
            "firewall",
            "kernel",
            "database schema",
            "delete data",
            "reconcile",
        ],
    ) {
        "high".to_string()
    } else if contains_any(
        task,
        &[
            "network",
            "protocol",
            "async",
            "concurrency",
            "refactor",
            "database",
            "storage",
            "parser",
            "serialization",
            "architecture",
            "dependency",
        ],
    ) {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

fn topology_for(risk: &str, department_count: usize, task: &str) -> &'static str {
    if risk == "low"
        && department_count == 0
        && contains_any(
            task,
            &["docs", "documentation", "typo", "comment", "readme"],
        )
    {
        "god_direct"
    } else if risk == "low" {
        "worker_verify"
    } else if risk == "medium" && department_count <= 2 {
        "worker_reviewer"
    } else if department_count >= 3 {
        "cross_cutting_parallel_then_integrate"
    } else {
        "plan_worker_gates_reviewer"
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn validate_risk(risk: &str) -> Result<()> {
    if !["low", "medium", "high", "critical"].contains(&risk) {
        bail!("risk must be low, medium, high, or critical");
    }
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn engineering_preset() -> OrchestrationConfig {
    OrchestrationConfig {
        schema: "codexflow.orchestration.v1".to_string(),
        independent_review: true,
        reviewer_role: "flow_reviewer".to_string(),
        default_worker_role: "flow_worker".to_string(),
        departments: vec![
            department(
                "security",
                "Security, trust boundaries, credentials, auth, and hostile input.",
                &[
                    "security",
                    "secret",
                    "credential",
                    "auth",
                    "tls",
                    "permission",
                    "untrusted",
                    "injection",
                    "vulnerability",
                ],
                &[
                    "security-threat-modeling",
                    "secrets-identity-access",
                    "application-security-assurance",
                ],
            ),
            department(
                "sre",
                "Reliability, service health, observability, capacity, and production operation.",
                &[
                    "production",
                    "sre",
                    "reliability",
                    "alert",
                    "metric",
                    "logging",
                    "monitoring",
                    "capacity",
                    "latency",
                    "uptime",
                ],
                &["production-sre", "observability-slo-engineering"],
            ),
            department(
                "infrastructure",
                "Hosts, deployment, provisioning, firewall, certificates, and environment.",
                &[
                    "deploy",
                    "vps",
                    "host",
                    "terraform",
                    "opentofu",
                    "ansible",
                    "systemd",
                    "firewall",
                    "certificate",
                    "infrastructure",
                ],
                &["infrastructure-as-code", "host-configuration-deployment"],
            ),
            department(
                "incident",
                "Failure, recovery, retries, crashes, disconnects, and incident handling.",
                &[
                    "failure",
                    "incident",
                    "crash",
                    "disconnect",
                    "retry",
                    "timeout",
                    "recovery",
                    "corrupt",
                    "queue full",
                    "outage",
                ],
                &["incident-response", "failure-chaos-engineering"],
            ),
            department(
                "architecture",
                "Authority, module/service boundaries, dependencies, and durable decisions.",
                &[
                    "architecture",
                    "crate",
                    "service boundary",
                    "authority",
                    "dependency",
                    "protocol",
                    "schema",
                    "ownership",
                    "adr",
                    "refactor",
                ],
                &["architecture-governance", "dependency-boundary-governance"],
            ),
            department(
                "data",
                "Dataset integrity, provenance, lineage, timestamps, schema, and corruption.",
                &[
                    "dataset",
                    "data",
                    "parquet",
                    "arrow",
                    "timestamp",
                    "lineage",
                    "provenance",
                    "schema",
                    "replay",
                ],
                &["data-reliability-governance", "data-lineage-contracts"],
            ),
            department(
                "quant",
                "Statistical validity, leakage, overfitting, walk-forward evidence, and research promotion.",
                &[
                    "strategy",
                    "detector",
                    "backtest",
                    "walk-forward",
                    "overfit",
                    "leakage",
                    "research",
                    "signal",
                    "slippage",
                    "profit",
                ],
                &["quant-research-validation", "quant-evidence-gate"],
            ),
            department(
                "devex",
                "Developer workflow, build speed, CI feedback, setup, and release ergonomics.",
                &[
                    "build",
                    "compile",
                    "cargo",
                    "ci",
                    "developer",
                    "setup",
                    "toolchain",
                    "release",
                    "cache",
                    "workflow",
                ],
                &["developer-experience", "build-release-engineering"],
            ),
        ],
    }
}

fn department(id: &str, description: &str, triggers: &[&str], skills: &[&str]) -> Department {
    Department {
        id: id.to_string(),
        description: description.to_string(),
        triggers: triggers.iter().map(|value| value.to_string()).collect(),
        skills: skills.iter().map(|value| value.to_string()).collect(),
        role: "flow_reviewer".to_string(),
        model: None,
        blocking_risks: vec!["high".to_string(), "critical".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_are_exclusive_until_released() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let first = acquire_lease(temp.path(), "task-demo", "worker-a", Some("demo"), 300)
            .expect("first lease");
        assert_eq!(first.owner, "worker-a");
        let error = acquire_lease(temp.path(), "task-demo", "worker-b", Some("demo"), 300)
            .expect_err("second owner must be blocked");
        assert!(error.to_string().contains("held by worker-a"));
        release_lease(temp.path(), "task-demo", "worker-a").expect("release");
        let second = acquire_lease(temp.path(), "task-demo", "worker-b", Some("demo"), 300)
            .expect("second lease");
        assert_eq!(second.owner, "worker-b");
    }

    #[test]
    fn expired_lease_can_be_reclaimed() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let dir = lease_dir(temp.path()).expect("lease dir");
        fs::create_dir_all(&dir).expect("lease dir create");
        let expired = LeaseRecord {
            schema: LEASE_SCHEMA.to_string(),
            scope: "task-demo".to_string(),
            owner: "worker-a".to_string(),
            task: Some("demo".to_string()),
            acquired_at: "2020-01-01T00:00:00Z".to_string(),
            renewed_at: "2020-01-01T00:00:00Z".to_string(),
            expires_at_ms: 1,
        };
        atomic_write_json(&lease_path(&dir, "task-demo"), &expired).expect("write expired");
        let lease = acquire_lease(temp.path(), "task-demo", "worker-b", Some("demo"), 300)
            .expect("reclaim");
        assert_eq!(lease.owner, "worker-b");
    }

    #[test]
    fn plan_emits_task_lease_scope() {
        let temp = tempfile::tempdir().expect("tempdir");
        let execution = plan(
            temp.path(),
            "update documentation".to_string(),
            None,
            Some("docs-task".to_string()),
        )
        .expect("plan");
        assert_eq!(execution.lease_scope.as_deref(), Some("task-docs-task"));
    }

    fn init_repo(root: &Path) {
        let status = Command::new("git")
            .current_dir(root)
            .args(["init", "-q"])
            .status()
            .expect("git init");
        assert!(status.success());
    }
}
