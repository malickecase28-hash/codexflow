use crate::runtime_state;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

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
