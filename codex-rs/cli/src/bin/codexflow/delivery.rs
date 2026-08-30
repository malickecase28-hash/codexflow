use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Args)]
pub struct DeliveryArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: DeliveryCommand,
}

#[derive(Debug, Subcommand)]
enum DeliveryCommand {
    Doctor,
    WorktreeCreate {
        #[arg(long)] task: String,
        #[arg(long, default_value = "main")] base: String,
        #[arg(long)] branch: Option<String>,
        #[arg(long)] path: Option<PathBuf>,
    },
    WorktreeRemove {
        #[arg(long)] task: String,
        #[arg(long)] path: Option<PathBuf>,
        #[arg(long)] delete_branch: bool,
        #[arg(long)] yes: bool,
    },
    WorktreeList,
    PrCreate {
        #[arg(long)] title: String,
        #[arg(long)] body: Option<String>,
        #[arg(long)] body_file: Option<PathBuf>,
        #[arg(long, default_value = "main")] base: String,
        #[arg(long)] draft: bool,
    },
    PrChecks { #[arg(long)] watch: bool },
    MergeCheck,
    Merge {
        #[arg(long)] yes: bool,
        #[arg(long)] auto: bool,
        #[arg(long, default_value = "squash")] method: String,
        #[arg(long)] delete_branch: bool,
    },
    Status,
}

#[derive(Debug, Serialize)]
struct DeliveryStatus {
    project_root: String,
    branch: String,
    dirty: bool,
    git: Option<String>,
    gh: Option<String>,
}

pub fn handle(project_root: &Path, args: DeliveryArgs) -> Result<()> {
    match args.command {
        DeliveryCommand::Doctor | DeliveryCommand::Status => status(project_root),
        DeliveryCommand::WorktreeCreate { task, base, branch, path } => worktree_create(project_root, &task, &base, branch.as_deref(), path.as_deref()),
        DeliveryCommand::WorktreeRemove { task, path, delete_branch, yes } => worktree_remove(project_root, &task, path.as_deref(), delete_branch, yes),
        DeliveryCommand::WorktreeList => run_git(project_root, &["worktree", "list"]),
        DeliveryCommand::PrCreate { title, body, body_file, base, draft } => pr_create(project_root, &title, body.as_deref(), body_file.as_deref(), &base, draft),
        DeliveryCommand::PrChecks { watch } => pr_checks(project_root, watch),
        DeliveryCommand::MergeCheck => merge_check(project_root),
        DeliveryCommand::Merge { yes, auto, method, delete_branch } => merge(project_root, yes, auto, &method, delete_branch),
    }
}

fn status(project_root: &Path) -> Result<()> {
    ensure_git_repo(project_root)?;
    let branch = git_output(project_root, &["branch", "--show-current"])?;
    let dirty = !git_output(project_root, &["status", "--porcelain"])?.trim().is_empty();
    let report = DeliveryStatus {
        project_root: project_root.display().to_string(),
        branch: branch.trim().to_string(),
        dirty,
        git: which::which("git").ok().map(|path| path.display().to_string()),
        gh: which::which("gh").ok().map(|path| path.display().to_string()),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn worktree_create(project_root: &Path, task: &str, base_ref: &str, branch: Option<&str>, explicit_path: Option<&Path>) -> Result<()> {
    ensure_git_repo(project_root)?;
    validate_task_id(task)?;
    let branch = branch.map(str::to_string).unwrap_or_else(|| format!("codexflow/task/{task}"));
    let path = explicit_path.map(Path::to_path_buf).unwrap_or_else(|| default_worktree_path(project_root, task));
    if path.exists() { bail!("worktree path already exists: {}", path.display()); }
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).with_context(|| format!("create worktree parent {}", parent.display()))?; }
    let status = Command::new("git").current_dir(project_root).arg("worktree").arg("add").arg("-b").arg(&branch).arg(&path).arg(base_ref).status().context("create git worktree")?;
    if !status.success() { bail!("git worktree add failed with {status}"); }
    println!("{}", serde_json::json!({"task": task, "branch": branch, "path": path, "base": base_ref}));
    Ok(())
}

fn worktree_remove(project_root: &Path, task: &str, explicit_path: Option<&Path>, delete_branch: bool, yes: bool) -> Result<()> {
    if !yes { bail!("worktree removal requires --yes"); }
    validate_task_id(task)?;
    let path = explicit_path.map(Path::to_path_buf).unwrap_or_else(|| default_worktree_path(project_root, task));
    let branch = format!("codexflow/task/{task}");
    let status = Command::new("git").current_dir(project_root).args(["worktree", "remove"]).arg(&path).status().context("remove git worktree")?;
    if !status.success() { bail!("git worktree remove failed with {status}"); }
    if delete_branch {
        let status = Command::new("git").current_dir(project_root).args(["branch", "-d", &branch]).status().context("delete task branch")?;
        if !status.success() { bail!("worktree removed but branch deletion failed with {status}"); }
    }
    Ok(())
}

fn pr_create(project_root: &Path, title: &str, body: Option<&str>, body_file: Option<&Path>, base: &str, draft: bool) -> Result<()> {
    require_gh()?;
    ensure_clean_commit_state(project_root)?;
    let mut command = Command::new("gh");
    command.current_dir(project_root).args(["pr", "create", "--title", title, "--base", base]);
    if draft { command.arg("--draft"); }
    match (body, body_file) {
        (Some(body), None) => { command.args(["--body", body]); }
        (None, Some(path)) => { command.arg("--body-file").arg(path); }
        (None, None) => { command.args(["--body", "Created by CodexFlow delivery plane."]); }
        (Some(_), Some(_)) => bail!("use --body or --body-file, not both"),
    }
    let status = command.status().context("create pull request")?;
    if !status.success() { bail!("gh pr create failed with {status}"); }
    Ok(())
}

fn pr_checks(project_root: &Path, watch: bool) -> Result<()> {
    require_gh()?;
    let mut command = Command::new("gh");
    command.current_dir(project_root).args(["pr", "checks", "--required"]);
    if watch { command.arg("--watch"); }
    let status = command.status().context("read pull-request checks")?;
    if !status.success() { bail!("required pull-request checks are not passing"); }
    Ok(())
}

fn merge_check(project_root: &Path) -> Result<()> {
    require_gh()?;
    ensure_clean_commit_state(project_root)?;
    let status = Command::new("gh").current_dir(project_root).args(["pr", "checks", "--required"]).status().context("check required pull-request checks")?;
    if !status.success() { bail!("merge blocked: required pull-request checks are not passing"); }
    let output = Command::new("gh").current_dir(project_root).args(["pr", "view", "--json", "number,state,isDraft,mergeStateStatus,reviewDecision,url"]).output().context("inspect pull request")?;
    if !output.status.success() { bail!("cannot inspect current pull request"); }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

fn merge(project_root: &Path, yes: bool, auto: bool, method: &str, delete_branch: bool) -> Result<()> {
    if !yes { bail!("merge requires --yes"); }
    if !["merge", "squash", "rebase"].contains(&method) { bail!("merge method must be merge, squash, or rebase"); }
    merge_check(project_root)?;
    let mut command = Command::new("gh");
    command.current_dir(project_root).args(["pr", "merge"]).arg(format!("--{method}"));
    if auto { command.arg("--auto"); }
    if delete_branch { command.arg("--delete-branch"); }
    let status = command.status().context("merge pull request")?;
    if !status.success() { bail!("gh pr merge failed with {status}"); }
    Ok(())
}

fn ensure_clean_commit_state(project_root: &Path) -> Result<()> {
    let status = git_output(project_root, &["status", "--porcelain"])?;
    if !status.trim().is_empty() { bail!("working tree is dirty; commit or stash before PR/merge operations"); }
    let branch = git_output(project_root, &["branch", "--show-current"])?;
    if branch.trim().is_empty() { bail!("detached HEAD cannot be delivered as a normal pull request"); }
    Ok(())
}

fn default_worktree_path(project_root: &Path, task: &str) -> PathBuf {
    let parent = project_root.parent().unwrap_or(project_root);
    let project_name = project_root.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "project".to_string());
    parent.join(".codexflow-worktrees").join(project_name).join(task)
}

fn ensure_git_repo(project_root: &Path) -> Result<()> {
    let status = Command::new("git").current_dir(project_root).args(["rev-parse", "--is-inside-work-tree"]).status().context("check git repository")?;
    if !status.success() { bail!("project root is not inside a git worktree"); }
    Ok(())
}
fn require_gh() -> Result<()> { which::which("gh").context("GitHub CLI `gh` is required for pull-request operations")?; Ok(()) }
fn git_output(project_root: &Path, args: &[&str]) -> Result<String> { let output = Command::new("git").current_dir(project_root).args(args).output().with_context(|| format!("git {}", args.join(" ")))?; if !output.status.success() { bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()); } Ok(String::from_utf8_lossy(&output.stdout).to_string()) }
fn run_git(project_root: &Path, args: &[&str]) -> Result<()> { let status = Command::new("git").current_dir(project_root).args(args).status().with_context(|| format!("git {}", args.join(" ")))?; if !status.success() { bail!("git {} failed with {status}", args.join(" ")); } Ok(()) }
fn validate_task_id(value: &str) -> Result<()> { if value.is_empty() || value.len() > 64 || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-') { bail!("invalid task id {value:?}"); } Ok(()) }
