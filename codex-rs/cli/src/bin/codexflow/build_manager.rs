use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::SecondsFormat;
use chrono::Utc;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Project id or unique name. Omit to resolve from the current directory.
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: BuildCommand,
}

#[derive(Debug, Subcommand)]
enum BuildCommand {
    Doctor,
    Configure {
        #[arg(long)]
        working_dir: Option<String>,
        #[arg(long)]
        target_dir: Option<String>,
        #[arg(long)]
        cache_mode: Option<String>,
        #[arg(long)]
        max_jobs: Option<u32>,
    },
    Check {
        #[arg(last = true, allow_hyphen_values = true)]
        cargo_args: Vec<OsString>,
    },
    Test {
        #[arg(last = true, allow_hyphen_values = true)]
        cargo_args: Vec<OsString>,
    },
    /// Run deterministic Rust verification and persist machine-readable evidence.
    Verify {
        /// Run cargo check only and skip cargo test.
        #[arg(long)]
        check_only: bool,
        #[arg(last = true, allow_hyphen_values = true)]
        cargo_args: Vec<OsString>,
    },
    Dev {
        #[arg(last = true, allow_hyphen_values = true)]
        cargo_args: Vec<OsString>,
    },
    Release {
        #[arg(long)]
        yes: bool,
        #[arg(last = true, allow_hyphen_values = true)]
        cargo_args: Vec<OsString>,
    },
    Timings {
        #[arg(long)]
        release: bool,
        #[arg(long)]
        yes: bool,
        #[arg(last = true, allow_hyphen_values = true)]
        cargo_args: Vec<OsString>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct BuildPolicy {
    working_dir: Option<String>,
    target_dir: Option<String>,
    cache_mode: String,
    max_jobs: Option<u32>,
    release_requires_confirmation: bool,
    timings_on_release: bool,
}

impl Default for BuildPolicy {
    fn default() -> Self {
        Self {
            working_dir: None,
            target_dir: None,
            cache_mode: "cargo".to_string(),
            max_jobs: None,
            release_requires_confirmation: true,
            timings_on_release: true,
        }
    }
}

#[derive(Debug, Serialize)]
struct VerificationStep {
    name: String,
    argv: Vec<String>,
    success: bool,
    exit_code: Option<i32>,
    duration_ms: u128,
}

#[derive(Debug, Serialize)]
struct VerificationReport {
    schema: &'static str,
    created_at: String,
    project_root: String,
    cargo_workdir: String,
    git_head: Option<String>,
    dirty_worktree: Option<bool>,
    steps: Vec<VerificationStep>,
    success: bool,
    evidence_path: String,
}

pub fn handle(project_root: &Path, args: BuildArgs) -> Result<()> {
    match args.command {
        BuildCommand::Doctor => doctor(project_root),
        BuildCommand::Configure {
            working_dir,
            target_dir,
            cache_mode,
            max_jobs,
        } => configure(project_root, working_dir, target_dir, cache_mode, max_jobs),
        BuildCommand::Check { cargo_args } => {
            run_cargo(project_root, "check", false, false, &cargo_args)
        }
        BuildCommand::Test { cargo_args } => {
            run_cargo(project_root, "test", false, false, &cargo_args)
        }
        BuildCommand::Verify {
            check_only,
            cargo_args,
        } => verify(project_root, check_only, &cargo_args),
        BuildCommand::Dev { cargo_args } => {
            run_cargo(project_root, "build", false, false, &cargo_args)
        }
        BuildCommand::Release { yes, cargo_args } => {
            let policy = load_policy(project_root)?;
            if policy.release_requires_confirmation && !yes {
                bail!(
                    "release build requires --yes; use cargo check/test during normal development"
                );
            }
            run_cargo(
                project_root,
                "build",
                true,
                policy.timings_on_release,
                &cargo_args,
            )
        }
        BuildCommand::Timings {
            release,
            yes,
            cargo_args,
        } => {
            if release && !yes {
                bail!("release timings require --yes because they perform a release build");
            }
            run_cargo(project_root, "build", release, true, &cargo_args)
        }
    }
}

pub fn apply_project_build_environment(project_root: &Path, command: &mut Command) -> Result<()> {
    let policy = load_policy(project_root)?;
    apply_policy_environment(project_root, &policy, command)
}

fn doctor(project_root: &Path) -> Result<()> {
    let policy = load_policy(project_root)?;
    let workdir = resolve_workdir(project_root, &policy)?;
    let report = serde_json::json!({
        "project_root": project_root,
        "cargo_workdir": workdir,
        "cargo": which::which("cargo").ok(),
        "rustc": which::which("rustc").ok(),
        "sccache": which::which("sccache").ok(),
        "policy": policy,
        "target_dir": resolved_target_dir(project_root, &policy),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn configure(
    project_root: &Path,
    working_dir: Option<String>,
    target_dir: Option<String>,
    cache_mode: Option<String>,
    max_jobs: Option<u32>,
) -> Result<()> {
    let mut policy = load_policy(project_root)?;
    if let Some(working_dir) = working_dir {
        policy.working_dir = if working_dir.trim().is_empty() {
            None
        } else {
            Some(working_dir)
        };
    }
    if let Some(target_dir) = target_dir {
        policy.target_dir = if target_dir.trim().is_empty() {
            None
        } else {
            Some(target_dir)
        };
    }
    if let Some(cache_mode) = cache_mode {
        if !["cargo", "sccache"].contains(&cache_mode.as_str()) {
            bail!("cache mode must be cargo or sccache");
        }
        if cache_mode == "sccache" && which::which("sccache").is_err() {
            bail!("sccache was requested but is not installed");
        }
        policy.cache_mode = cache_mode;
    }
    if max_jobs.is_some() {
        policy.max_jobs = max_jobs;
    }
    save_policy(project_root, &policy)?;
    println!("{}", serde_json::to_string_pretty(&policy)?);
    Ok(())
}

fn verify(project_root: &Path, check_only: bool, cargo_args: &[OsString]) -> Result<()> {
    let policy = load_policy(project_root)?;
    let workdir = resolve_workdir(project_root, &policy)?;
    let created_at = Utc::now();
    let git_head = git_output_optional(project_root, &["rev-parse", "HEAD"]);
    let dirty_worktree = git_output_optional(
        project_root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )
    .map(|status| !status.trim().is_empty());

    let mut steps = Vec::new();
    let check = run_cargo_evidence(project_root, "check", false, false, cargo_args)?;
    let check_passed = check.success;
    steps.push(check);
    if check_passed && !check_only {
        steps.push(run_cargo_evidence(
            project_root,
            "test",
            false,
            false,
            cargo_args,
        )?);
    }
    let success = steps.iter().all(|step| step.success);
    let evidence_path = verification_path(project_root, created_at.timestamp_millis(), git_head.as_deref());
    let report = VerificationReport {
        schema: "codexflow.build-verification.v1",
        created_at: created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        project_root: project_root.display().to_string(),
        cargo_workdir: workdir.display().to_string(),
        git_head,
        dirty_worktree,
        steps,
        success,
        evidence_path: evidence_path.display().to_string(),
    };
    save_verification_report(&evidence_path, &report)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !success {
        bail!(
            "verification failed; machine-readable evidence was written to {}",
            evidence_path.display()
        );
    }
    Ok(())
}

fn run_cargo(
    project_root: &Path,
    subcommand: &str,
    release: bool,
    timings: bool,
    cargo_args: &[OsString],
) -> Result<()> {
    let mut command = cargo_command(project_root, subcommand, release, timings, cargo_args)?;
    eprintln!("CodexFlow build: {:?}", command);
    let status = command.status().context("run cargo")?;
    if !status.success() {
        bail!("cargo {subcommand} exited with status {status}");
    }
    Ok(())
}

fn run_cargo_evidence(
    project_root: &Path,
    subcommand: &str,
    release: bool,
    timings: bool,
    cargo_args: &[OsString],
) -> Result<VerificationStep> {
    let mut command = cargo_command(project_root, subcommand, release, timings, cargo_args)?;
    let mut argv = vec![command.get_program().to_string_lossy().into_owned()];
    argv.extend(
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned()),
    );
    eprintln!("CodexFlow verify: {:?}", command);
    let started = Instant::now();
    let status = command.status().with_context(|| format!("run cargo {subcommand}"))?;
    Ok(VerificationStep {
        name: format!("cargo_{subcommand}"),
        argv,
        success: status.success(),
        exit_code: status.code(),
        duration_ms: started.elapsed().as_millis(),
    })
}

fn cargo_command(
    project_root: &Path,
    subcommand: &str,
    release: bool,
    timings: bool,
    cargo_args: &[OsString],
) -> Result<Command> {
    let policy = load_policy(project_root)?;
    let workdir = resolve_workdir(project_root, &policy)?;
    let mut command = Command::new("cargo");
    command.current_dir(&workdir).arg(subcommand);
    if release {
        command.arg("--release");
    }
    if timings {
        command.arg("--timings");
    }
    if let Some(jobs) = policy.max_jobs {
        command.arg("--jobs").arg(jobs.to_string());
    }
    command.args(cargo_args);
    apply_policy_environment(project_root, &policy, &mut command)?;
    Ok(command)
}

fn apply_policy_environment(
    project_root: &Path,
    policy: &BuildPolicy,
    command: &mut Command,
) -> Result<()> {
    if let Some(target_dir) = resolved_target_dir(project_root, policy) {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    if policy.cache_mode == "sccache" {
        which::which("sccache")
            .context("sccache cache mode selected but sccache is not installed")?;
        command.env("RUSTC_WRAPPER", "sccache");
        command.env("CARGO_INCREMENTAL", "0");
    }
    if let Some(jobs) = policy.max_jobs {
        command.env("CARGO_BUILD_JOBS", jobs.to_string());
    }
    Ok(())
}

fn verification_path(project_root: &Path, timestamp_ms: i64, git_head: Option<&str>) -> PathBuf {
    let revision = git_head
        .map(|head| head.chars().take(12).collect::<String>())
        .filter(|head| !head.is_empty())
        .unwrap_or_else(|| "nogit".to_string());
    project_root
        .join(".codexflow")
        .join("evidence")
        .join("build")
        .join(format!("{timestamp_ms}-{revision}.json"))
}

fn save_verification_report(path: &Path, report: &VerificationReport) -> Result<()> {
    fs::create_dir_all(path.parent().context("verification evidence parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(report)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    if cfg!(windows) && path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))
}

fn git_output_optional(project_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(project_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn policy_path(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("build.json")
}

fn load_policy(project_root: &Path) -> Result<BuildPolicy> {
    let path = policy_path(project_root);
    if !path.exists() {
        return Ok(BuildPolicy::default());
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

fn save_policy(project_root: &Path, policy: &BuildPolicy) -> Result<()> {
    let path = policy_path(project_root);
    fs::create_dir_all(path.parent().context("build policy parent")?)?;
    fs::write(&path, serde_json::to_vec_pretty(policy)?)
        .with_context(|| format!("write {}", path.display()))
}

fn resolve_workdir(project_root: &Path, policy: &BuildPolicy) -> Result<PathBuf> {
    if let Some(value) = policy.working_dir.as_deref() {
        let path = project_root.join(value);
        if path.join("Cargo.toml").is_file() {
            return Ok(path);
        }
        bail!(
            "configured Cargo working directory has no Cargo.toml: {}",
            path.display()
        );
    }
    if project_root.join("Cargo.toml").is_file() {
        return Ok(project_root.to_path_buf());
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(project_root).context("scan project root for Cargo workspace")? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("Cargo.toml").is_file() {
            candidates.push(entry.path());
        }
    }

    match candidates.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!(
            "no Cargo.toml found at the project root or an unambiguous top-level directory; configure --working-dir"
        ),
        _ => bail!(
            "multiple top-level Cargo workspaces detected; configure --working-dir explicitly"
        ),
    }
}

fn resolved_target_dir(project_root: &Path, policy: &BuildPolicy) -> Option<PathBuf> {
    let configured = policy.target_dir.as_deref()?;
    let path = PathBuf::from(configured);
    Some(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_path_is_bounded_and_revision_scoped() {
        let root = Path::new("/tmp/project");
        let path = verification_path(
            root,
            1234,
            Some("0123456789abcdef0123456789abcdef01234567"),
        );
        assert!(path.ends_with(".codexflow/evidence/build/1234-0123456789ab.json"));
    }

    #[test]
    fn build_policy_defaults_to_incremental_cargo_mode() {
        let policy = BuildPolicy::default();
        assert_eq!(policy.cache_mode, "cargo");
        assert!(!policy.release_requires_confirmation.eq(&false));
    }
}
