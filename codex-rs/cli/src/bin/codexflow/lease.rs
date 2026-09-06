use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::SecondsFormat;
use chrono::Utc;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const LEASE_SCHEMA: &str = "codexflow.lease.v1";
const MIN_TTL_SECONDS: u64 = 30;
const MAX_TTL_SECONDS: u64 = 86_400;
const METADATA_RETRY_COUNT: usize = 4;
const METADATA_RETRY_DELAY: Duration = Duration::from_millis(8);
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(10);
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Args)]
pub struct LeaseArgs {
    #[command(subcommand)]
    command: LeaseCommand,
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

pub fn handle(project_root: &Path, args: LeaseArgs) -> Result<()> {
    match args.command {
        LeaseCommand::Acquire {
            scope,
            owner,
            task,
            ttl_seconds,
        } => {
            let lease = acquire(project_root, &scope, &owner, task.as_deref(), ttl_seconds)?;
            println!("{}", serde_json::to_string_pretty(&lease)?);
            Ok(())
        }
        LeaseCommand::Renew {
            scope,
            owner,
            ttl_seconds,
        } => {
            let lease = renew(project_root, &scope, &owner, ttl_seconds)?;
            println!("{}", serde_json::to_string_pretty(&lease)?);
            Ok(())
        }
        LeaseCommand::Release { scope, owner } => {
            release(project_root, &scope, &owner)?;
            println!("{scope}");
            Ok(())
        }
        LeaseCommand::List => {
            println!("{}", serde_json::to_string_pretty(&list(project_root)?)?);
            Ok(())
        }
        LeaseCommand::Prune => {
            println!("{}", serde_json::to_string_pretty(&prune(project_root)?)?);
            Ok(())
        }
    }
}

pub fn task_scope(task_id: &str) -> Result<String> {
    validate_token(task_id, "task id")?;
    Ok(format!("task-{task_id}"))
}

fn acquire(
    project_root: &Path,
    scope: &str,
    owner: &str,
    task: Option<&str>,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    validate_token(scope, "lease scope")?;
    let base = lease_base_dir(project_root)?;
    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    let _guard = ScopeMutationLock::acquire(&base, scope)?;
    acquire_unlocked(project_root, scope, owner, task, ttl_seconds)
}

fn acquire_unlocked(
    project_root: &Path,
    scope: &str,
    owner: &str,
    task: Option<&str>,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    validate_token(scope, "lease scope")?;
    validate_token(owner, "lease owner")?;
    if let Some(task) = task {
        validate_token(task, "task id")?;
    }
    let ttl_seconds = validate_ttl(ttl_seconds)?;
    let base = lease_base_dir(project_root)?;
    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    let scope_dir = scope_dir(&base, scope);

    for _ in 0..4 {
        match fs::create_dir(&scope_dir) {
            Ok(()) => {
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
                if let Err(error) = atomic_write_record(&record_path(&scope_dir), &lease) {
                    let _ = fs::remove_dir_all(&scope_dir);
                    return Err(error).context("initialize newly acquired lease");
                }
                return Ok(lease);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = load_record_retry(&scope_dir).with_context(|| {
                    format!(
                        "lease {scope} exists but its metadata is unavailable; refusing to steal ownership"
                    )
                })?;
                if existing.owner == owner && !expired(&existing) {
                    return renew_unlocked(project_root, scope, owner, ttl_seconds);
                }
                if expired(&existing) {
                    match fs::remove_dir_all(&scope_dir) {
                        Ok(()) => continue,
                        Err(remove_error)
                            if remove_error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            continue;
                        }
                        Err(remove_error) => {
                            return Err(remove_error).with_context(|| {
                                format!("remove expired lease {}", scope_dir.display())
                            });
                        }
                    }
                }
                bail!(
                    "lease {scope} is held by {} until {}",
                    existing.owner,
                    existing.expires_at_ms
                );
            }
            Err(error) => {
                return Err(error).with_context(|| format!("create {}", scope_dir.display()));
            }
        }
    }
    bail!("lease {scope} changed repeatedly while acquiring; retry the operation")
}

fn renew(project_root: &Path, scope: &str, owner: &str, ttl_seconds: u64) -> Result<LeaseRecord> {
    validate_token(scope, "lease scope")?;
    let base = lease_base_dir(project_root)?;
    let _guard = ScopeMutationLock::acquire(&base, scope)?;
    renew_unlocked(project_root, scope, owner, ttl_seconds)
}

fn renew_unlocked(
    project_root: &Path,
    scope: &str,
    owner: &str,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    validate_token(scope, "lease scope")?;
    validate_token(owner, "lease owner")?;
    let ttl_seconds = validate_ttl(ttl_seconds)?;
    let base = lease_base_dir(project_root)?;
    let scope_dir = scope_dir(&base, scope);
    let mut lease = load_record_retry(&scope_dir)?;
    if lease.owner != owner {
        bail!("lease {scope} is owned by {}, not {owner}", lease.owner);
    }
    if expired(&lease) {
        bail!("lease {scope} has expired; acquire it again instead of renewing it");
    }
    let now = Utc::now();
    lease.renewed_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    lease.expires_at_ms = now
        .timestamp_millis()
        .saturating_add(ttl_millis(ttl_seconds));
    atomic_write_record(&record_path(&scope_dir), &lease)?;
    Ok(lease)
}

fn release(project_root: &Path, scope: &str, owner: &str) -> Result<()> {
    validate_token(scope, "lease scope")?;
    let base = lease_base_dir(project_root)?;
    let _guard = ScopeMutationLock::acquire(&base, scope)?;
    release_unlocked(project_root, scope, owner)
}

fn release_unlocked(project_root: &Path, scope: &str, owner: &str) -> Result<()> {
    validate_token(scope, "lease scope")?;
    validate_token(owner, "lease owner")?;
    let base = lease_base_dir(project_root)?;
    let scope_dir = scope_dir(&base, scope);
    let lease = load_record_retry(&scope_dir)?;
    if lease.owner != owner {
        bail!("lease {scope} is owned by {}, not {owner}", lease.owner);
    }
    fs::remove_dir_all(&scope_dir).with_context(|| format!("remove {}", scope_dir.display()))
}

fn list(project_root: &Path) -> Result<Vec<LeaseView>> {
    let base = lease_base_dir(project_root)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut leases = Vec::new();
    for entry in fs::read_dir(&base).with_context(|| format!("read {}", base.display()))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.path().is_dir() {
            continue;
        }
        let lease = match load_record_retry(&entry.path()) {
            Ok(lease) => lease,
            Err(_) => continue,
        };
        leases.push(LeaseView {
            expired: expired(&lease),
            lease,
        });
    }
    leases.sort_by(|left, right| left.lease.scope.cmp(&right.lease.scope));
    Ok(leases)
}

fn prune(project_root: &Path) -> Result<Vec<String>> {
    let base = lease_base_dir(project_root)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for entry in fs::read_dir(&base).with_context(|| format!("read {}", base.display()))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let scope_dir = entry.path();
        if !scope_dir.is_dir() {
            continue;
        }
        let Some(scope_name) = scope_dir
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if validate_token(&scope_name, "lease scope").is_err() {
            continue;
        }
        let _guard = ScopeMutationLock::acquire(&base, &scope_name)?;
        if !scope_dir.is_dir() {
            continue;
        }
        let lease = match load_record_retry(&scope_dir) {
            Ok(lease) => lease,
            Err(_) => continue,
        };
        if lease.scope != scope_name {
            continue;
        }
        if expired(&lease) {
            match fs::remove_dir_all(&scope_dir) {
                Ok(()) => removed.push(lease.scope),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("remove {}", scope_dir.display()));
                }
            }
        }
    }
    removed.sort();
    Ok(removed)
}

struct ScopeMutationLock {
    _file: File,
}

impl ScopeMutationLock {
    fn acquire(base: &Path, scope: &str) -> Result<Self> {
        let lock_root = base.join(".locks");
        fs::create_dir_all(&lock_root)
            .with_context(|| format!("create {}", lock_root.display()))?;
        let path = lock_root.join(format!("{scope}.lock"));
        if path.is_dir() {
            bail!(
                "legacy lease mutation lock directory {}; remove it after confirming no older CodexFlow process is active",
                path.display()
            );
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("open lease mutation lock {}", path.display()))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => {
                    if started.elapsed() > LOCK_WAIT_LIMIT {
                        bail!(
                            "timed out waiting for lease mutation lock {}",
                            path.display()
                        );
                    }
                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error)
                        .with_context(|| format!("lock lease mutation state {}", path.display()));
                }
            }
        }
    }
}

fn lease_base_dir(project_root: &Path) -> Result<PathBuf> {
    let output = git_output(project_root, &["rev-parse", "--git-common-dir"])?;
    let raw = output.trim();
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

fn scope_dir(base: &Path, scope: &str) -> PathBuf {
    base.join(scope)
}

fn record_path(scope_dir: &Path) -> PathBuf {
    scope_dir.join("lease.json")
}

fn load_record_retry(scope_dir: &Path) -> Result<LeaseRecord> {
    let path = record_path(scope_dir);
    let mut last_error = None;
    for attempt in 0..METADATA_RETRY_COUNT {
        match load_record(&path) {
            Ok(record) => return Ok(record),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < METADATA_RETRY_COUNT {
                    thread::sleep(METADATA_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.context("lease metadata retry produced no error")?)
}

fn load_record(path: &Path) -> Result<LeaseRecord> {
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let lease: LeaseRecord =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    if lease.schema != LEASE_SCHEMA {
        bail!("unsupported lease schema {}", lease.schema);
    }
    Ok(lease)
}

fn expired(lease: &LeaseRecord) -> bool {
    Utc::now().timestamp_millis() >= lease.expires_at_ms
}

fn validate_ttl(ttl_seconds: u64) -> Result<u64> {
    if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl_seconds) {
        bail!("lease ttl must be between {MIN_TTL_SECONDS} and {MAX_TTL_SECONDS} seconds");
    }
    Ok(ttl_seconds)
}

fn ttl_millis(ttl_seconds: u64) -> i64 {
    i64::try_from(ttl_seconds)
        .unwrap_or(i64::MAX / 1000)
        .saturating_mul(1000)
}

fn validate_token(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        bail!("invalid {label} {value:?}; use lowercase letters, digits, _ or -, max 64 chars");
    }
    Ok(())
}

fn atomic_write_record(path: &Path, record: &LeaseRecord) -> Result<()> {
    fs::create_dir_all(path.parent().context("lease record parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(record)?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn scope_directory_is_exclusive_until_release() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let first =
            acquire(temp.path(), "task-demo", "worker-a", Some("demo"), 300).expect("first lease");
        assert_eq!(first.owner, "worker-a");
        let error = acquire(temp.path(), "task-demo", "worker-b", Some("demo"), 300)
            .expect_err("second owner must be blocked");
        assert!(error.to_string().contains("held by worker-a"));
        release(temp.path(), "task-demo", "worker-a").expect("release");
        let second =
            acquire(temp.path(), "task-demo", "worker-b", Some("demo"), 300).expect("second lease");
        assert_eq!(second.owner, "worker-b");
    }

    #[test]
    fn expired_scope_directory_can_be_reclaimed() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let base = lease_base_dir(temp.path()).expect("base");
        let scope = scope_dir(&base, "task-demo");
        fs::create_dir_all(&scope).expect("scope dir");
        let expired_record = LeaseRecord {
            schema: LEASE_SCHEMA.to_string(),
            scope: "task-demo".to_string(),
            owner: "worker-a".to_string(),
            task: Some("demo".to_string()),
            acquired_at: "2020-01-01T00:00:00Z".to_string(),
            renewed_at: "2020-01-01T00:00:00Z".to_string(),
            expires_at_ms: 1,
        };
        atomic_write_record(&record_path(&scope), &expired_record).expect("expired record");
        let lease =
            acquire(temp.path(), "task-demo", "worker-b", Some("demo"), 300).expect("reclaim");
        assert_eq!(lease.owner, "worker-b");
    }

    #[test]
    fn renew_keeps_scope_directory_as_authority() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        acquire(temp.path(), "task-demo", "worker-a", None, 300).expect("acquire");
        let base = lease_base_dir(temp.path()).expect("base");
        let scope = scope_dir(&base, "task-demo");
        let renewed = renew(temp.path(), "task-demo", "worker-a", 600).expect("renew");
        assert!(scope.is_dir());
        assert_eq!(renewed.owner, "worker-a");
    }

    #[test]
    fn scope_mutation_lock_serializes_same_scope() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let base = lease_base_dir(temp.path()).expect("base");
        fs::create_dir_all(&base).expect("base dir");
        let first = ScopeMutationLock::acquire(&base, "task-demo").expect("first lock");
        let worker_base = base.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready_tx.send(()).expect("signal ready");
            let _second =
                ScopeMutationLock::acquire(&worker_base, "task-demo").expect("second lock");
            acquired_tx.send(()).expect("signal acquired");
        });

        ready_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("worker ready");
        assert!(
            acquired_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        drop(first);
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("worker acquired after release");
        worker.join().expect("worker join");
    }

    #[test]
    fn previous_owner_cannot_release_reacquired_scope() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let base = lease_base_dir(temp.path()).expect("base");
        let scope = scope_dir(&base, "task-demo");
        fs::create_dir_all(&scope).expect("scope dir");
        let expired_record = LeaseRecord {
            schema: LEASE_SCHEMA.to_string(),
            scope: "task-demo".to_string(),
            owner: "worker-a".to_string(),
            task: Some("demo".to_string()),
            acquired_at: "2020-01-01T00:00:00Z".to_string(),
            renewed_at: "2020-01-01T00:00:00Z".to_string(),
            expires_at_ms: 1,
        };
        atomic_write_record(&record_path(&scope), &expired_record).expect("expired record");
        let current =
            acquire(temp.path(), "task-demo", "worker-b", Some("demo"), 300).expect("reacquire");
        assert_eq!(current.owner, "worker-b");
        let error = release(temp.path(), "task-demo", "worker-a")
            .expect_err("previous owner must not release current lease");
        assert!(error.to_string().contains("owned by worker-b"));
        let persisted = load_record_retry(&scope).expect("current record");
        assert_eq!(persisted.owner, "worker-b");
    }

    #[test]
    fn task_scope_validates_task_id() {
        assert_eq!(task_scope("build-1").expect("scope"), "task-build-1");
        assert!(task_scope("Bad Task").is_err());
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
