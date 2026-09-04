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
use std::fs::TryLockError;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const RECOVERY_EVENT_SCHEMA: &str = "codexflow.recovery-event.v1";
const LOCK_TIMEOUT: Duration = Duration::from_secs(8);
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(25);
const MAX_HISTORY_LIMIT: usize = 1_000;

pub(super) fn append(project_root: &Path, decision: &RecoveryDecision) -> Result<()> {
    let state_dir = recovery_state_dir(project_root);
    fs::create_dir_all(&state_dir)
        .with_context(|| format!("create {}", state_dir.display()))?;
    let _guard = RecoveryLock::acquire(&state_dir)?;

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
    let _guard = RecoveryLock::acquire(&state_dir)?;

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
    _file: File,
}

impl RecoveryLock {
    fn acquire(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join(".recovery.lock");
        if path.is_dir() {
            bail!(
                "legacy recovery lock directory {}; remove it after confirming no older CodexFlow process is active",
                path.display()
            );
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("open recovery lock {}", path.display()))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => {
                    file.set_len(0)
                        .with_context(|| format!("truncate recovery lock {}", path.display()))?;
                    writeln!(
                        file,
                        "pid={} acquired_at={}",
                        std::process::id(),
                        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
                    )
                    .with_context(|| format!("write recovery lock {}", path.display()))?;
                    file.sync_data()
                        .with_context(|| format!("sync recovery lock {}", path.display()))?;
                    return Ok(Self { _file: file });
                }
                Err(TryLockError::WouldBlock) => {
                    if started.elapsed() >= LOCK_TIMEOUT {
                        bail!("timed out waiting for recovery lock {}", path.display());
                    }
                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error)
                        .with_context(|| format!("lock recovery state {}", path.display()));
                }
            }
        }
    }
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
    fn recovery_lock_is_released_when_handle_drops() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state_dir = recovery_state_dir(temp.path());
        fs::create_dir_all(&state_dir).expect("state dir");
        let guard = RecoveryLock::acquire(&state_dir).expect("lock");
        let path = state_dir.join(".recovery.lock");
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("second handle");
        assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
        drop(guard);
        second.try_lock().expect("lock is released after drop");
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
