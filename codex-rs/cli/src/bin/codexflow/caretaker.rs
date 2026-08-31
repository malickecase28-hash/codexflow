use crate::runtime_state;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const WORKFLOW_TEMPLATE: &str = include_str!("caretaker-workflow.yml");

#[derive(Debug, Args)]
pub struct CaretakerArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: CaretakerCommand,
}

#[derive(Debug, Subcommand)]
enum CaretakerCommand {
    Init {
        #[arg(long)]
        force: bool,
    },
    Show,
    Scan {
        #[arg(long)]
        json: bool,
    },
    Queue {
        #[arg(long)]
        apply: bool,
    },
    WorkflowInstall {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct CaretakerPolicy {
    schema: String,
    enabled: bool,
    auto_fix: bool,
    max_source_file_bytes: u64,
    todo_threshold: usize,
    duplicate_basename_threshold: usize,
    allowed_queue_risks: Vec<String>,
    one_change_per_pr: bool,
}

impl Default for CaretakerPolicy {
    fn default() -> Self {
        Self {
            schema: "codexflow.caretaker.v1".to_string(),
            enabled: true,
            auto_fix: false,
            max_source_file_bytes: 200_000,
            todo_threshold: 20,
            duplicate_basename_threshold: 3,
            allowed_queue_risks: vec!["low".to_string(), "medium".to_string()],
            one_change_per_pr: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Finding {
    id: String,
    kind: String,
    risk: String,
    summary: String,
    paths: Vec<String>,
    recommendation: String,
}

#[derive(Debug, Serialize)]
struct ScanReport {
    schema: &'static str,
    project_root: String,
    clean_worktree: bool,
    tracked_files: usize,
    findings: Vec<Finding>,
}

pub fn handle(project_root: &Path, args: CaretakerArgs) -> Result<()> {
    match args.command {
        CaretakerCommand::Init { force } => init(project_root, force),
        CaretakerCommand::Show => {
            println!(
                "{}",
                serde_json::to_string_pretty(&load_policy(project_root)?)?
            );
            Ok(())
        }
        CaretakerCommand::Scan { json } => {
            let report = scan(project_root)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_human(&report);
            }
            Ok(())
        }
        CaretakerCommand::Queue { apply } => queue(project_root, apply),
        CaretakerCommand::WorkflowInstall { force } => install_workflow(project_root, force),
    }
}

fn init(project_root: &Path, force: bool) -> Result<()> {
    let path = policy_path(project_root);
    if path.exists() && !force {
        bail!(
            "caretaker policy already exists at {}; use --force to replace it",
            path.display()
        );
    }
    fs::create_dir_all(path.parent().context("caretaker policy parent")?)?;
    fs::write(
        &path,
        serde_json::to_vec_pretty(&CaretakerPolicy::default())?,
    )
    .with_context(|| format!("write {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

fn scan(project_root: &Path) -> Result<ScanReport> {
    let policy = load_policy(project_root)?;
    let status = git_output(project_root, &["status", "--porcelain"])?;
    let clean_worktree = status.trim().is_empty();
    let tracked = git_output(project_root, &["ls-files", "-z"])?;
    let files = tracked
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut findings = Vec::new();

    if !clean_worktree {
        findings.push(finding("dirty_worktree", "high", "Repository has uncommitted changes; autonomous maintenance should not start from this checkout.", Vec::new(), "Finish, commit, or isolate the existing work before caretaker execution."));
    }

    let mut basenames: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut todo_hits: BTreeMap<String, usize> = BTreeMap::new();
    for relative in &files {
        let path = project_root.join(relative);
        if !path.is_file() {
            continue;
        }
        if is_source_path(relative) {
            if let Ok(metadata) = fs::metadata(&path)
                && metadata.len() > policy.max_source_file_bytes
            {
                findings.push(finding("large_source_file", "medium", &format!("Source file exceeds caretaker size threshold: {} bytes.", metadata.len()), vec![relative.display().to_string()], "Review whether the file contains multiple responsibilities or generated content before splitting it."));
            }
            if let Ok(text) = fs::read_to_string(&path) {
                let count = text
                    .lines()
                    .filter(|line| line.contains("TODO") || line.contains("FIXME"))
                    .count();
                if count >= policy.todo_threshold {
                    todo_hits.insert(relative.display().to_string(), count);
                }
            }
        }
        if let Some(name) = relative.file_name() {
            basenames
                .entry(name.to_string_lossy().to_ascii_lowercase())
                .or_default()
                .push(relative.display().to_string());
        }
    }

    for (path, count) in todo_hits {
        findings.push(finding("todo_density", "low", &format!("{count} TODO/FIXME markers are concentrated in one tracked file."), vec![path], "Triage markers into real tasks, remove obsolete notes, and keep actionable debt visible."));
    }
    for (name, paths) in basenames {
        if paths.len() >= policy.duplicate_basename_threshold && !is_common_basename(&name) {
            findings.push(finding("duplicate_basename", "low", &format!("{} tracked files share basename {name}; inspect for semantic duplication before adding another.", paths.len()), paths, "Use semantic deduplication and authority analysis; matching names alone do not prove duplication."));
        }
    }

    if has_rust_project(project_root)
        && !project_root.join(".codexflow").join("build.json").exists()
    {
        findings.push(finding("rust_build_policy_missing", "low", "Rust project has no CodexFlow build-cost policy.", Vec::new(), "Configure a persistent target directory and preferred cache mode before expensive build work."));
    }
    if !project_root
        .join(".codexflow")
        .join("orchestration.json")
        .exists()
        && !project_root
            .join("docs")
            .join("maintenance")
            .join("departments.json")
            .exists()
    {
        findings.push(finding("orchestration_policy_missing", "low", "Project has no explicit CodexFlow orchestration manifest.", Vec::new(), "Initialize a minimal or engineering orchestration manifest before autonomous multi-agent changes."));
    }

    findings.sort_by(|left, right| {
        risk_rank(&right.risk)
            .cmp(&risk_rank(&left.risk))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(ScanReport {
        schema: "codexflow.caretaker.scan.v1",
        project_root: project_root.display().to_string(),
        clean_worktree,
        tracked_files: files.len(),
        findings,
    })
}

fn queue(project_root: &Path, apply: bool) -> Result<()> {
    let policy = load_policy(project_root)?;
    let report = scan(project_root)?;
    if !report.clean_worktree {
        bail!("caretaker queue is blocked by a dirty worktree");
    }
    let mut queued = Vec::new();
    for finding in report.findings {
        if !policy.allowed_queue_risks.contains(&finding.risk) {
            continue;
        }
        let task_id = format!("maint_{}", finding.id);
        if apply {
            let gates = if finding.risk == "medium" {
                vec!["independent_review".to_string()]
            } else {
                Vec::new()
            };
            runtime_state::seed_orchestration_plan(
                project_root,
                &task_id,
                &finding.summary,
                &finding.risk,
                &gates,
            )?;
        }
        queued.push(serde_json::json!({"task_id": task_id, "finding": finding, "applied": apply}));
    }
    println!("{}", serde_json::to_string_pretty(&queued)?);
    Ok(())
}

fn install_workflow(project_root: &Path, force: bool) -> Result<()> {
    let path = project_root
        .join(".github")
        .join("workflows")
        .join("codexflow-caretaker.yml");
    if path.exists() && !force {
        bail!(
            "workflow already exists at {}; use --force to replace it",
            path.display()
        );
    }
    fs::create_dir_all(path.parent().context("workflow parent")?)?;
    fs::write(&path, WORKFLOW_TEMPLATE).with_context(|| format!("write {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

fn policy_path(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("caretaker.json")
}
fn load_policy(project_root: &Path) -> Result<CaretakerPolicy> {
    let path = policy_path(project_root);
    if !path.exists() {
        return Ok(CaretakerPolicy::default());
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

fn finding(
    kind: &str,
    risk: &str,
    summary: &str,
    paths: Vec<String>,
    recommendation: &str,
) -> Finding {
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    summary.hash(&mut hasher);
    paths.hash(&mut hasher);
    Finding {
        id: format!("{:08x}", hasher.finish() as u32),
        kind: kind.to_string(),
        risk: risk.to_string(),
        summary: summary.to_string(),
        paths,
        recommendation: recommendation.to_string(),
    }
}

fn print_human(report: &ScanReport) {
    println!("project: {}", report.project_root);
    println!("tracked files: {}", report.tracked_files);
    println!("clean worktree: {}", report.clean_worktree);
    if report.findings.is_empty() {
        println!("findings: none");
        return;
    }
    println!("findings:");
    for finding in &report.findings {
        println!(
            "  [{}] {} {} - {}",
            finding.risk, finding.id, finding.kind, finding.summary
        );
        for path in &finding.paths {
            println!("      {path}");
        }
    }
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
fn is_source_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some(
            "rs" | "py"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "go"
                | "java"
                | "kt"
                | "c"
                | "cc"
                | "cpp"
                | "h"
                | "hpp"
                | "cs"
                | "swift"
        )
    )
}
fn is_common_basename(name: &str) -> bool {
    matches!(
        name,
        "mod.rs"
            | "lib.rs"
            | "main.rs"
            | "index.ts"
            | "index.tsx"
            | "index.js"
            | "readme.md"
            | "license"
            | "package.json"
            | "cargo.toml"
    )
}
fn has_rust_project(project_root: &Path) -> bool {
    if project_root.join("Cargo.toml").is_file() {
        return true;
    }
    fs::read_dir(project_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| entry.path().is_dir() && entry.path().join("Cargo.toml").is_file())
}
fn risk_rank(risk: &str) -> u8 {
    match risk {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}
