#[path = "codexflow/build_manager.rs"]
mod build_manager;
#[path = "codexflow/caretaker.rs"]
mod caretaker;
#[path = "codexflow/delivery.rs"]
mod delivery;
#[path = "codexflow/orchestrator.rs"]
mod orchestrator;
#[path = "codexflow/runtime.rs"]
mod runtime_state;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use codex_core::config::find_codex_home;
use codex_state::Project;
use codex_state::ProjectRoot;
use codex_state::ProjectSortKey;
use codex_state::SortDirection;
use codex_state::SqliteConfig;
use codex_state::StateRuntime;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::canonicalize_existing_preserving_symlinks;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const GOD_INSTRUCTIONS: &str = include_str!("codexflow_god.md");
const MANAGED_KEY: &str = "codexflow.managed";
const SCHEMA_KEY: &str = "codexflow.schema";
const SCHEMA_VERSION: &str = "2";
const PROJECT_ID_ENV: &str = "CODEXFLOW_PROJECT_ID";
const PROJECT_NAME_ENV: &str = "CODEXFLOW_PROJECT_NAME";

const FLOW_ROLES: &[(&str, &str)] = &[
    (
        "flow_explorer.toml",
        r#"name = "flow_explorer"
description = "Read-only investigation agent for a bounded question about the active project."
developer_instructions = """
Investigate only the delegated question. Do not edit files. Read the minimum useful context, identify authoritative paths and evidence, and return concise findings with file references, risks, and unresolved questions. Do not propose broad rewrites unless the evidence requires one.
"""
"#,
    ),
    (
        "flow_worker.toml",
        r#"name = "flow_worker"
description = "Implementation agent for a bounded code change with explicit ownership."
developer_instructions = """
Execute only the delegated objective inside the assigned project roots and write scope. Assume other agents may be editing elsewhere. Do not revert unrelated changes. Implement the smallest correct change and run focused checks. Do not approve your own review or specialist gates. Return changed paths, checks, unresolved risks, and concise handoff facts.
"""
"#,
    ),
    (
        "flow_verifier.toml",
        r#"name = "flow_verifier"
description = "Verification agent for tests, reproduction, static checks, and evidence collection."
developer_instructions = """
Verify the stated acceptance criteria independently. Prefer running checks and inspecting evidence over rewriting implementation. Do not make production edits unless the root explicitly delegates a verification-only fixture or test correction. Return exact commands, results, failures, and confidence limits.
"""
"#,
    ),
    (
        "flow_reviewer.toml",
        r#"name = "flow_reviewer"
description = "Independent reviewer for completed changes with no implementation ownership."
developer_instructions = """
Review the supplied diff and acceptance criteria independently. Do not inherit or defend the implementer's reasoning. Look for correctness, security, concurrency, data-loss, compatibility, performance, test, and maintainability defects relevant to the active project. Rank findings by severity and cite exact paths. Do not edit implementation code unless explicitly reassigned after review.
"""
"#,
    ),
    (
        "flow_integrator.toml",
        r#"name = "flow_integrator"
description = "Integration agent for reconciling completed disjoint changes and validating the combined result."
developer_instructions = """
Integrate only completed, reviewed work owned by disjoint workers. Resolve mechanical conflicts without silently changing domain semantics. Run integration checks after combining changes. Escalate semantic conflicts to the root rather than choosing an authority by convenience.
"""
"#,
    ),
];

#[derive(Debug, Parser)]
#[command(
    name = "codexflow",
    version,
    about = "Project-aware Codex orchestration harness"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<TopCommand>,
}

#[derive(Debug, Subcommand)]
enum TopCommand {
    Project(ProjectArgs),
    Run(RunArgs),
    Doctor(DoctorArgs),
    Setup(SetupArgs),
    Runtime(runtime_state::RuntimeArgs),
    Build(build_manager::BuildArgs),
    Orchestrate(orchestrator::OrchestrateArgs),
    Delivery(delivery::DeliveryArgs),
    Caretaker(caretaker::CaretakerArgs),
}

#[derive(Debug, Args)]
struct ProjectArgs {
    #[command(subcommand)]
    command: ProjectCommand,
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Add {
        name: String,
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Current {
        #[arg(long)]
        json: bool,
    },
    Show {
        target: String,
        #[arg(long)]
        json: bool,
    },
    Rename {
        target: String,
        name: String,
    },
    RootAdd {
        target: String,
        path: PathBuf,
    },
    RootRemove {
        target: String,
        path: PathBuf,
    },
    Delete {
        target: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Args)]
struct RunArgs {
    project: Option<String>,
    #[arg(last = true, allow_hyphen_values = true)]
    codex_args: Vec<OsString>,
}
#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
}
#[derive(Debug, Args)]
struct SetupArgs {
    #[arg(long)]
    force: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(TopCommand::Setup(args)) => setup(args),
        command => {
            let runtime = open_state_runtime().await?;
            match command {
                Some(TopCommand::Project(args)) => handle_project(&runtime, args).await,
                Some(TopCommand::Run(args)) => run_project(&runtime, args).await,
                Some(TopCommand::Doctor(args)) => doctor(&runtime, args).await,
                Some(TopCommand::Runtime(args)) => {
                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;
                    runtime_state::handle(&primary_root(&project)?, args)
                }
                Some(TopCommand::Build(args)) => {
                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;
                    build_manager::handle(&primary_root(&project)?, args)
                }
                Some(TopCommand::Orchestrate(args)) => {
                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;
                    orchestrator::handle(&primary_root(&project)?, args)
                }
                Some(TopCommand::Delivery(args)) => {
                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;
                    delivery::handle(&primary_root(&project)?, args)
                }
                Some(TopCommand::Caretaker(args)) => {
                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;
                    caretaker::handle(&primary_root(&project)?, args)
                }
                Some(TopCommand::Setup(_)) => unreachable!(),
                None => {
                    run_project(
                        &runtime,
                        RunArgs {
                            project: None,
                            codex_args: Vec::new(),
                        },
                    )
                    .await
                }
            }
        }
    }
}

async fn open_state_runtime() -> Result<std::sync::Arc<StateRuntime>> {
    let cwd = AbsolutePathBuf::current_dir().context("resolve current directory")?;
    let codex_home = find_codex_home().context("resolve CODEX_HOME")?;
    let sqlite_home = match std::env::var("CODEX_SQLITE_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => AbsolutePathBuf::resolve_path_against_base(value.trim(), &cwd),
        None => codex_home,
    };
    StateRuntime::init(
        SqliteConfig::from_sqlite_home(sqlite_home),
        "openai".to_string(),
    )
    .await
    .context("initialize Codex SQLite state")
}

fn setup(args: SetupArgs) -> Result<()> {
    let codex_home = find_codex_home().context("resolve CODEX_HOME")?;
    let agents_dir = codex_home.join("agents");
    fs::create_dir_all(&agents_dir).with_context(|| format!("create {}", agents_dir.display()))?;
    for (name, content) in FLOW_ROLES {
        let path = agents_dir.join(name);
        if path.exists() && !args.force {
            continue;
        }
        fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }
    let profile = codex_home.join("codexflow.config.toml");
    if !profile.exists() || args.force {
        let profile_text = format!(
            "developer_instructions = {god:?}\n\n[features]\nmulti_agent = true\n\n[agents]\nmax_threads = 8\nmax_depth = 2\n",
            god = GOD_INSTRUCTIONS
        );
        fs::write(&profile, profile_text)
            .with_context(|| format!("write {}", profile.display()))?;
    }
    println!("CodexFlow profile: {}", profile.display());
    println!("Generic roles: {}", agents_dir.display());
    Ok(())
}

async fn handle_project(runtime: &StateRuntime, args: ProjectArgs) -> Result<()> {
    match args.command {
        ProjectCommand::Add { name, roots } => {
            let roots = if roots.is_empty() {
                vec![std::env::current_dir().context("read current directory")?]
            } else {
                roots
            };
            let roots = canonical_roots(roots)?;
            let key = idempotency_key(&roots)?;
            if let Some(project) = runtime
                .get_project_by_idempotency_key(&key)
                .await
                .context("read project idempotency key")?
            {
                print_project(&project, false)?;
                eprintln!("Project already existed for the same idempotency key.");
                return Ok(());
            }
            ensure_roots_not_registered(runtime, &roots).await?;
            let mut metadata = BTreeMap::new();
            metadata.insert(MANAGED_KEY.to_string(), "true".to_string());
            metadata.insert(SCHEMA_KEY.to_string(), SCHEMA_VERSION.to_string());
            let project_roots = roots
                .iter()
                .cloned()
                .map(|path| ProjectRoot { path })
                .collect::<Vec<_>>();
            let created = runtime
                .create_project(name, project_roots, metadata, &[], &key)
                .await
                .context("create project")?;
            print_project(&created.project, false)?;
            if !created.created {
                eprintln!("Project already existed for the same idempotency key.");
            }
            Ok(())
        }
        ProjectCommand::List { json } => {
            let projects = all_projects(runtime).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else if projects.is_empty() {
                println!("No Codex projects are registered.");
            } else {
                for project in projects {
                    print_project(&project, false)?;
                }
            }
            Ok(())
        }
        ProjectCommand::Current { json } => {
            let project = current_project(runtime).await?;
            print_project(&project, json)
        }
        ProjectCommand::Show { target, json } => {
            let project = find_project(runtime, &target).await?;
            print_project(&project, json)
        }
        ProjectCommand::Rename { target, name } => {
            let project = find_project(runtime, &target).await?;
            let (updated, _) = runtime
                .update_project(&project.id, Some(name), None, None)
                .await
                .context("rename project")?
                .context("project disappeared during rename")?;
            print_project(&updated, false)
        }
        ProjectCommand::RootAdd { target, path } => {
            let project = find_project(runtime, &target).await?;
            let new_root = canonical_root(&path)?;
            if project
                .roots
                .iter()
                .any(|root| roots_equal(&root.path, &new_root))
            {
                bail!("root is already registered on project {}", project.name);
            }
            ensure_roots_not_registered(runtime, std::slice::from_ref(&new_root)).await?;
            let mut roots = project.roots.clone();
            roots.push(ProjectRoot { path: new_root });
            let (updated, _) = runtime
                .update_project(&project.id, None, Some(roots), None)
                .await
                .context("add project root")?
                .context("project disappeared while adding root")?;
            print_project(&updated, false)
        }
        ProjectCommand::RootRemove { target, path } => {
            let project = find_project(runtime, &target).await?;
            if project.roots.len() == 1 {
                bail!("cannot remove the last project root");
            }
            let remove = canonical_root(&path)?;
            let roots = project
                .roots
                .iter()
                .filter(|root| !roots_equal(&root.path, &remove))
                .cloned()
                .collect::<Vec<_>>();
            if roots.len() == project.roots.len() {
                bail!("root is not registered on project {}", project.name);
            }
            let (updated, _) = runtime
                .update_project(&project.id, None, Some(roots), None)
                .await
                .context("remove project root")?
                .context("project disappeared while removing root")?;
            print_project(&updated, false)
        }
        ProjectCommand::Delete { target, yes } => {
            if !yes {
                bail!("project delete requires --yes; project files are never deleted");
            }
            let project = find_project(runtime, &target).await?;
            runtime
                .delete_project(&project.id)
                .await
                .context("delete project record")?
                .context("project disappeared during delete")?;
            println!("Removed project record: {} ({})", project.name, project.id);
            Ok(())
        }
    }
}

async fn run_project(runtime: &StateRuntime, args: RunArgs) -> Result<()> {
    let project = resolve_scoped_project(runtime, args.project.as_deref()).await?;
    let primary_root = primary_root(&project)?;
    let codex = sibling_codex_executable()?;
    let encoded_god = serde_json::to_string(GOD_INSTRUCTIONS)?;
    let mut command = Command::new(&codex);
    command
        .arg("--profile")
        .arg("codexflow")
        .arg("--enable")
        .arg("multi_agent")
        .arg("-C")
        .arg(&primary_root)
        .arg("-c")
        .arg(format!("developer_instructions={encoded_god}"))
        .args(args.codex_args)
        .env(PROJECT_ID_ENV, &project.id)
        .env(PROJECT_NAME_ENV, &project.name);
    build_manager::apply_project_build_environment(&primary_root, &mut command)?;
    let status = command
        .status()
        .with_context(|| format!("launch {}", codex.display()))?;
    if !status.success() {
        bail!("Codex exited with status {status}");
    }
    Ok(())
}

async fn doctor(runtime: &StateRuntime, args: DoctorArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read current directory")?;
    let current = current_project(runtime).await.ok();
    let codex = sibling_codex_executable().ok();
    let report = serde_json::json!({"cwd": cwd, "project": current, "codex": codex, "project_env": std::env::var(PROJECT_ID_ENV).ok(), "sccache": which::which("sccache").ok(), "cargo": which::which("cargo").ok(), "rustc": which::which("rustc").ok(), "gh": which::which("gh").ok()});
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}
async fn resolve_scoped_project(runtime: &StateRuntime, target: Option<&str>) -> Result<Project> {
    match target {
        Some(target) => find_project(runtime, target).await,
        None => current_project(runtime).await,
    }
}
fn primary_root(project: &Project) -> Result<PathBuf> {
    Ok(PathBuf::from(
        &project
            .roots
            .first()
            .context("managed project has no roots")?
            .path,
    ))
}
async fn all_projects(runtime: &StateRuntime) -> Result<Vec<Project>> {
    let mut cursor = None;
    let mut projects = Vec::new();
    loop {
        let page = runtime
            .list_projects(
                cursor.as_deref(),
                100,
                ProjectSortKey::Position,
                SortDirection::Asc,
            )
            .await
            .context("list projects")?;
        projects.extend(page.projects);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(projects)
}
async fn find_project(runtime: &StateRuntime, target: &str) -> Result<Project> {
    if let Some(project) = runtime.get_project(target).await.context("read project")? {
        return Ok(project);
    }
    let matches = all_projects(runtime)
        .await?
        .into_iter()
        .filter(|project| project.name.eq_ignore_ascii_case(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [project] => Ok(project.clone()),
        [] => bail!("project not found: {target}"),
        many => bail!(
            "project name is ambiguous: {target}; matching ids: {}",
            many.iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
async fn current_project(runtime: &StateRuntime) -> Result<Project> {
    let cwd = canonicalize_existing_preserving_symlinks(
        &std::env::current_dir().context("read current directory")?,
    )
    .context("canonicalize current directory")?;
    let mut matches = Vec::new();
    for project in all_projects(runtime).await? {
        for root in &project.roots {
            let root_path = PathBuf::from(&root.path);
            let canonical =
                canonicalize_existing_preserving_symlinks(&root_path).unwrap_or(root_path);
            if cwd.starts_with(&canonical) {
                matches.push((canonical.components().count(), project.clone()));
            }
        }
    }
    matches.sort_by(|left, right| right.0.cmp(&left.0));
    let best = matches.first().cloned().context("current directory is not inside a managed CodexFlow project; run `codexflow project add <name>`")?;
    if matches
        .iter()
        .skip(1)
        .any(|candidate| candidate.0 == best.0 && candidate.1.id != best.1.id)
    {
        bail!("current directory matches multiple equally specific projects");
    }
    Ok(best.1)
}
async fn ensure_roots_not_registered(runtime: &StateRuntime, roots: &[String]) -> Result<()> {
    for project in all_projects(runtime).await? {
        for existing in &project.roots {
            if roots
                .iter()
                .any(|candidate| roots_equal(&existing.path, candidate))
            {
                bail!(
                    "root is already registered to project {} ({})",
                    project.name,
                    project.id
                );
            }
        }
    }
    Ok(())
}
fn canonical_roots(paths: Vec<PathBuf>) -> Result<Vec<String>> {
    let mut roots: Vec<String> = Vec::new();
    for path in paths {
        let root = canonical_root(&path)?;
        if !roots.iter().any(|existing| roots_equal(existing, &root)) {
            roots.push(root);
        }
    }
    if roots.is_empty() {
        bail!("at least one project root is required");
    }
    Ok(roots)
}
fn canonical_root(path: &Path) -> Result<String> {
    let absolute = AbsolutePathBuf::resolve_path_against_base(
        path,
        std::env::current_dir().context("read current directory")?,
    );
    if !absolute.as_path().is_dir() {
        bail!(
            "project root is not an existing directory: {}",
            absolute.display()
        );
    }
    let canonical = canonicalize_existing_preserving_symlinks(absolute.as_path())
        .with_context(|| format!("canonicalize project root {}", absolute.display()))?;
    Ok(canonical.to_string_lossy().into_owned())
}
fn roots_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}
fn idempotency_key(roots: &[String]) -> Result<String> {
    let first = roots.first().context("project has no root")?;
    let key = format!("codexflow:project:{first}");
    if key.len() > 512 {
        bail!("project root is too long to form a stable idempotency key");
    }
    Ok(key)
}
fn print_project(project: &Project, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(project)?);
        return Ok(());
    }
    println!("{} ({})", project.name, project.id);
    for root in &project.roots {
        println!("  {}", root.path);
    }
    Ok(())
}
fn sibling_codex_executable() -> Result<PathBuf> {
    let current = std::env::current_exe().context("resolve codexflow executable")?;
    let sibling_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(parent) = current.parent() {
        let sibling = parent.join(sibling_name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    let path = which::which("codex").context("find codex executable on PATH")?;
    if path == current {
        bail!("resolved codex executable points back to codexflow");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roots_equal_is_platform_appropriate() {
        if cfg!(windows) {
            assert!(roots_equal(r"C:\Code\Demo", r"c:\code\demo"));
        } else {
            assert!(!roots_equal("/Code/Demo", "/code/demo"));
        }
    }
    #[test]
    fn idempotency_key_uses_primary_root() {
        let roots = vec!["/repo".to_string(), "/repo-extra".to_string()];
        assert_eq!(
            idempotency_key(&roots).expect("idempotency key"),
            "codexflow:project:/repo"
        );
    }
}
