use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSpec {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub required: bool,
    pub timeout_ms: u64,
}

impl ProcessSpec {
    pub fn new(name: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            program: program.into(),
            args: Vec::new(),
            required: true,
            timeout_ms: 5_000,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub const fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub const fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Succeeded,
    Failed,
    Unavailable,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessEvidence {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub required: bool,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ProcessEvidence {
    pub const fn passed(&self) -> bool {
        matches!(self.status, ProcessStatus::Succeeded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitEnvironment {
    pub branch: String,
    pub head: String,
    pub changed_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub working_directory: PathBuf,
    pub require_git: bool,
    pub probes: Vec<ProcessSpec>,
    pub baseline_checks: Vec<ProcessSpec>,
    pub max_command_output_bytes: usize,
}

impl BootstrapRequest {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            working_directory: working_directory.into(),
            require_git: true,
            probes: Vec::new(),
            baseline_checks: Vec::new(),
            max_command_output_bytes: 8 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapSnapshot {
    pub working_directory: PathBuf,
    pub operating_system: String,
    pub architecture: String,
    pub git: Option<GitEnvironment>,
    pub probes: Vec<ProcessEvidence>,
    pub baseline_checks: Vec<ProcessEvidence>,
    pub ready: bool,
}

impl BootstrapSnapshot {
    pub fn render_summary(&self) -> String {
        let mut lines = vec![
            format!("workspace: {}", self.working_directory.display()),
            format!("platform: {}/{}", self.operating_system, self.architecture),
            format!("ready: {}", self.ready),
        ];
        if let Some(git) = &self.git {
            lines.push(format!("git: {} @ {}", git.branch, git.head));
            if !git.changed_files.is_empty() {
                lines.push(format!(
                    "changed: {}",
                    git.changed_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        } else {
            lines.push("git: unavailable".to_string());
        }
        for evidence in self.probes.iter().chain(&self.baseline_checks) {
            lines.push(format!(
                "{}: {:?}{}",
                evidence.name,
                evidence.status,
                if evidence.required { " [required]" } else { "" }
            ));
        }
        lines.join("\n")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("bootstrap working directory does not exist: {0}")]
    MissingWorkingDirectory(PathBuf),
    #[error("bootstrap working directory is not a directory: {0}")]
    NotDirectory(PathBuf),
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentBootstrapper;

impl EnvironmentBootstrapper {
    pub async fn inspect(
        &self,
        request: &BootstrapRequest,
    ) -> Result<BootstrapSnapshot, BootstrapError> {
        if !request.working_directory.exists() {
            return Err(BootstrapError::MissingWorkingDirectory(
                request.working_directory.clone(),
            ));
        }
        if !request.working_directory.is_dir() {
            return Err(BootstrapError::NotDirectory(
                request.working_directory.clone(),
            ));
        }

        let git = inspect_git(request).await;
        let mut probes = Vec::with_capacity(request.probes.len());
        for spec in &request.probes {
            probes.push(run_spec(request, spec).await);
        }
        let mut baseline_checks = Vec::with_capacity(request.baseline_checks.len());
        for spec in &request.baseline_checks {
            baseline_checks.push(run_spec(request, spec).await);
        }

        let required_processes_pass = probes
            .iter()
            .chain(&baseline_checks)
            .filter(|evidence| evidence.required)
            .all(ProcessEvidence::passed);
        let ready = required_processes_pass && (!request.require_git || git.is_some());

        Ok(BootstrapSnapshot {
            working_directory: request.working_directory.clone(),
            operating_system: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            git,
            probes,
            baseline_checks,
            ready,
        })
    }
}

async fn inspect_git(request: &BootstrapRequest) -> Option<GitEnvironment> {
    let branch = run_git(request, ["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let head = run_git(request, ["rev-parse", "HEAD"]).await?;
    let tracked = run_git(request, ["diff", "--name-only", "-z", "HEAD"])
        .await
        .unwrap_or_default();
    let untracked = run_git(
        request,
        ["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await
    .unwrap_or_default();
    let changed_files = nul_paths(&tracked)
        .chain(nul_paths(&untracked))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    Some(GitEnvironment {
        branch: branch.trim().to_string(),
        head: head.trim().to_string(),
        changed_files,
    })
}

async fn run_git<const N: usize>(request: &BootstrapRequest, args: [&str; N]) -> Option<String> {
    let spec = ProcessSpec::new("git", "git")
        .with_args(args)
        .with_timeout_ms(3_000);
    let evidence = run_spec(request, &spec).await;
    evidence.passed().then_some(evidence.stdout)
}

async fn run_spec(request: &BootstrapRequest, spec: &ProcessSpec) -> ProcessEvidence {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&request.working_directory)
        .kill_on_drop(true);
    let duration = Duration::from_millis(spec.timeout_ms.max(1));
    let result = timeout(duration, command.output()).await;

    match result {
        Err(_) => process_evidence(spec, ProcessStatus::TimedOut, None, String::new(), String::new()),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => process_evidence(
            spec,
            ProcessStatus::Unavailable,
            None,
            String::new(),
            bounded_text(&error.to_string(), request.max_command_output_bytes),
        ),
        Ok(Err(error)) => process_evidence(
            spec,
            ProcessStatus::Failed,
            None,
            String::new(),
            bounded_text(&error.to_string(), request.max_command_output_bytes),
        ),
        Ok(Ok(output)) => {
            let status = if output.status.success() {
                ProcessStatus::Succeeded
            } else {
                ProcessStatus::Failed
            };
            process_evidence(
                spec,
                status,
                output.status.code(),
                bounded_bytes(&output.stdout, request.max_command_output_bytes),
                bounded_bytes(&output.stderr, request.max_command_output_bytes),
            )
        }
    }
}

fn process_evidence(
    spec: &ProcessSpec,
    status: ProcessStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
) -> ProcessEvidence {
    ProcessEvidence {
        name: spec.name.clone(),
        program: spec.program.clone(),
        args: spec.args.clone(),
        required: spec.required,
        status,
        exit_code,
        stdout,
        stderr,
    }
}

fn bounded_bytes(bytes: &[u8], max_bytes: usize) -> String {
    bounded_text(&String::from_utf8_lossy(bytes), max_bytes)
}

fn bounded_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

fn nul_paths(value: &str) -> impl Iterator<Item = PathBuf> + '_ {
    value
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_truncation_is_utf8_safe() {
        let text = "abc😀def";
        let bounded = bounded_text(text, 5);
        assert_eq!(bounded, "abc\n[truncated]");
    }

    #[test]
    fn nul_paths_are_dedicated_repository_entries() {
        let paths = nul_paths("src/a.rs\0tests/a.rs\0").collect::<Vec<_>>();
        assert_eq!(paths, vec![PathBuf::from("src/a.rs"), PathBuf::from("tests/a.rs")]);
    }

    #[test]
    fn bootstrap_defaults_require_git_and_bound_output() {
        let request = BootstrapRequest::new(PathBuf::from("/workspace/project"));
        assert!(request.require_git);
        assert_eq!(request.max_command_output_bytes, 8 * 1024);
    }

    #[test]
    fn optional_probe_does_not_change_required_flag_back() {
        let spec = ProcessSpec::new("optional", "tool").optional();
        assert!(!spec.required);
    }
}
