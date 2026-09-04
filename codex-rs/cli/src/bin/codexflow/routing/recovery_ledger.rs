use super::RecoveryDecision;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::SecondsFormat;
use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

const RECOVERY_EVENT_SCHEMA: &str = "codexflow.recovery-event.v1";
const LOCK_STALE_AFTER: Duration = Duration::from_secs(120);
const LOCK_TIMEOUT: Duration = Duration::from_secs(8);
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(25);
const MAX_HISTORY_LIMIT: usize = 1_000;
static LOCK_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) fn append(project_root: &Path, decision: &RecoveryDecision) -> Result<()> {
    let state_dir = recovery_state_dir(project_root);
    fs::create_dir_all(&state_dir)
        .with_context(|| format!("create {}", state_dir.display()))?;
    let mut guard = RecoveryLock::acquire(&state_dir)?;
    guard.refresh_and_assert_owned()?;

    let event = serde_json::json!({
        "schema": RECOVERY_EVENT_SCHEMA,
        "recorded_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "decision": decision,
    });
    let mut encoded = serde_json::to_vec(&event)?;
    encoded.push(b'\n');

    let path = recovery_history_path(project_root);
    repair_torn_tail(&path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(&encoded)
        .with_context(|| format!("append {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

pub(super) fn history(project_root: &Path, limit: usize) -> Result<Vec<Value>> {
    if limit == 0 || limit > MAX_HISTORY_LIMIT {
        bail!("history limit must be between 1 and {MAX_HISTORY_LIMIT}");
    }

    let state_dir = recovery_state_dir(project_root);
    fs::create_dir_all(&state_dir)
        .with_context(|| format!("create {}", state_dir.display()))?;
    let mut guard = RecoveryLock::acquire(&state_dir)?;
    guard.refresh_and_assert_owned()?;

    let path = recovery_history_path(project_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    repair_torn_tail(&path)?;

    let data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in data.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = serde_json::from_slice(line)
            .with_context(|| format!("parse line {} from {}", index + 1, path.display()))?;
        validate_event(&value)
            .with_context(|| format!("validate line {} from {}", index + 1, path.display()))?;
        records.push(value);
    }

    let start = records.len().saturating_sub(limit);
    Ok(records.split_off(start))
}

fn repair_torn_tail(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let data = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if data.is_empty() || data.ends_with(b"\n") {
        return Ok(());
    }

    let tail_start = data
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let tail = &data[tail_start..];
    match serde_json::from_slice::<Value>(tail) {
        Ok(value) if validate_event(&value).is_ok() => {
            let mut file = OpenOptions::new()
                .append(true)
                .open(path)
                .with_context(|| format!("open {}", path.display()))?;
            file.write_all(b"\n")
                .with_context(|| format!("complete newline in {}", path.display()))?;
            file.sync_data()
                .with_context(|| format!("sync {}", path.display()))?;
        }
        Ok(_) => {
            // A syntactically complete but invalid record is genuine corruption.
            // Leave it intact so the normal history parser reports it.
        }
        Err(error) if error.is_eof() => {
            let file = OpenOptions::new()
                .write(true)
                .open(path)
                .with_context(|| format!("open {}", path.display()))?;
            file.set_len(tail_start as u64)
                .with_context(|| format!("truncate torn tail in {}", path.display()))?;
            file.sync_data()
                .with_context(|| format!("sync {}", path.display()))?;
        }
        Err(_) => {
            // Non-EOF JSON errors are not unambiguously torn writes.
            // Preserve the bytes so callers see the corruption instead of losing data.
        }
    }
    Ok(())
}

fn validate_event(value: &Value) -> Result<()> {
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .context("recovery event is missing schema")?;
    if schema != RECOVERY_EVENT_SCHEMA {
        bail!("unsupported recovery event schema {schema}");
    }
    if value.get("recorded_at").and_then(Value::as_str).is_none() {
        bail!("recovery event is missing recorded_at");
    }
    if value.get("decision").and_then(Value::as_object).is_none() {
        bail!("recovery event is missing decision object");
    }
    Ok(())
}

fn recovery_state_dir(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("state")
}

fn recovery_history_path(project_root: &Path) -> PathBuf {
    recovery_state_dir(project_root).join("recovery-v1.jsonl")
}

struct RecoveryLock {
    path: PathBuf,
    file: File,
    token: String,
}

impl RecoveryLock {
    fn acquire(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join(".recovery.lock");
        let token = format!(
            "{}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros(),
            LOCK_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let started = Instant::now();

        loop {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(token.as_bytes())
                        .with_context(|| format!("initialize recovery lock {}", path.display()))?;
                    file.sync_data()
                        .with_context(|| format!("sync recovery lock {}", path.display()))?;
                    return Ok(Self {
                        path,
                        file,
                        token: token.clone(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if path.is_dir() {
                        if lock_is_stale(&path) {
                            bail!(
                                "stale legacy recovery lock {}; remove it manually after confirming no older CodexFlow process is active",
                                path.display()
                            );
                        }
                    } else if try_reap_stale_lock(&path)? {
                        continue;
                    }
                    if started.elapsed() >= LOCK_TIMEOUT {
                        bail!("timed out waiting for recovery lock {}", path.display());
                    }
                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("create recovery lock {}", path.display()));
                }
            }
        }
    }

    fn refresh_and_assert_owned(&mut self) -> Result<()> {
        self.file
            .seek(SeekFrom::Start(0))
            .with_context(|| format!("seek recovery lock {}", self.path.display()))?;
        self.file
            .set_len(0)
            .with_context(|| format!("truncate recovery lock {}", self.path.display()))?;
        self.file
            .write_all(self.token.as_bytes())
            .with_context(|| format!("refresh recovery lock {}", self.path.display()))?;
        self.file
            .sync_data()
            .with_context(|| format!("sync recovery lock {}", self.path.display()))?;

        let current = fs::read_to_string(&self.path)
            .with_context(|| format!("verify recovery lock {}", self.path.display()))?;
        if current != self.token {
            bail!("recovery lock ownership changed for {}", self.path.display());
        }
        Ok(())
    }
}

impl Drop for RecoveryLock {
    fn drop(&mut self) {
        let still_owned = fs::read_to_string(&self.path).is_ok_and(|token| token == self.token);
        if still_owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn try_reap_stale_lock(path: &Path) -> Result<bool> {
    let first_modified = match stale_modified_time(path)? {
        Some(modified) => modified,
        None => return Ok(false),
    };
    let first_token = match fs::read_to_string(path) {
        Ok(token) => token,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };

    let second_modified = match stale_modified_time(path)? {
        Some(modified) if modified == first_modified => modified,
        _ => return Ok(false),
    };
    let second_token = match fs::read_to_string(path) {
        Ok(token) => token,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    if second_token != first_token {
        return Ok(false);
    }

    let third_modified = match stale_modified_time(path)? {
        Some(modified) if modified == second_modified => modified,
        _ => return Ok(false),
    };
    if third_modified != first_modified {
        return Ok(false);
    }

    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error).with_context(|| format!("remove stale lock {}", path.display())),
    }
}

fn stale_modified_time(path: &Path) -> Result<Option<SystemTime>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("read modified time for {}", path.display()))?;
    let stale = modified
        .elapsed()
        .is_ok_and(|elapsed| elapsed >= LOCK_STALE_AFTER);
    Ok(stale.then_some(modified))
}

fn lock_is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed >= LOCK_STALE_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_decision(root: &Path) -> RecoveryDecision {
        super::super::resolve_recovery(
            root,
            super::super::FAILURE_TEST,
            2,
            Some(super::super::PROFILE_DEEP),
            Some("compiler evidence".to_string()),
        )
        .expect("recovery decision")
    }

    #[test]
    fn empty_history_is_valid() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(history(temp.path(), 10).expect("history").is_empty());
    }

    #[test]
    fn history_rejects_unbounded_limits() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(history(temp.path(), 0).is_err());
        assert!(history(temp.path(), MAX_HISTORY_LIMIT + 1).is_err());
    }

    #[test]
    fn round_trips_real_recovery_decision() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = sample_decision(temp.path());
        append(temp.path(), &decision).expect("append recovery event");
        let records = history(temp.path(), 10).expect("history");
        assert_eq!(records.len(), 1);
        let stored = &records[0]["decision"];
        assert_eq!(stored["failure_class"], "test");
        assert_eq!(stored["next_profile"], "critical");
        assert_eq!(stored["rollback_recommended"], true);
    }

    #[test]
    fn repairs_only_an_incomplete_final_jsonl_tail() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = sample_decision(temp.path());
        append(temp.path(), &decision).expect("append recovery event");
        let path = recovery_history_path(temp.path());
        let mut file = OpenOptions::new().append(true).open(&path).expect("open history");
        file.write_all(b"{\"schema\":").expect("write torn tail");
        file.sync_data().expect("sync torn tail");

        let records = history(temp.path(), 10).expect("history repairs tail");
        assert_eq!(records.len(), 1);
        let repaired = fs::read(&path).expect("read repaired history");
        assert!(repaired.ends_with(b"\n"));
        assert!(!repaired.ends_with(b"{\"schema\":"));
    }

    #[test]
    fn preserves_complete_record_missing_only_final_newline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = sample_decision(temp.path());
        append(temp.path(), &decision).expect("append recovery event");
        let path = recovery_history_path(temp.path());
        let file = OpenOptions::new().write(true).open(&path).expect("open history");
        let len = file.metadata().expect("history metadata").len();
        file.set_len(len - 1).expect("remove final newline");
        file.sync_data().expect("sync history");

        let records = history(temp.path(), 10).expect("history completes newline");
        assert_eq!(records.len(), 1);
        assert!(fs::read(&path).expect("history bytes").ends_with(b"\n"));
    }

    #[test]
    fn does_not_hide_non_tail_corruption() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state_dir = recovery_state_dir(temp.path());
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::write(recovery_history_path(temp.path()), b"{not-json}\n").expect("write corruption");
        assert!(history(temp.path(), 10).is_err());
    }

    #[test]
    fn recovery_lock_drop_preserves_replacement_token() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state_dir = recovery_state_dir(temp.path());
        fs::create_dir_all(&state_dir).expect("state dir");
        let guard = RecoveryLock::acquire(&state_dir).expect("lock");
        let path = guard.path.clone();
        fs::remove_file(&path).expect("simulate replacement");
        fs::write(&path, "replacement-token").expect("replacement token");
        drop(guard);
        assert_eq!(
            fs::read_to_string(&path).expect("replacement survives"),
            "replacement-token"
        );
    }

    #[test]
    fn validates_recovery_event_shape() {
        let valid = serde_json::json!({
            "schema": RECOVERY_EVENT_SCHEMA,
            "recorded_at": "2026-09-04T00:00:00.000Z",
            "decision": {"schema": "codexflow.recovery-decision.v1"},
        });
        assert!(validate_event(&valid).is_ok());
        assert!(validate_event(&serde_json::json!({"schema": "wrong"})).is_err());
    }
}
