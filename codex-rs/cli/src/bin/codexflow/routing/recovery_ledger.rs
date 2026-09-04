use super::RecoveryDecision;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::SecondsFormat;
use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const RECOVERY_EVENT_SCHEMA: &str = "codexflow.recovery-event.v1";
const LOCK_STALE_AFTER: Duration = Duration::from_secs(120);
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
    let path = recovery_history_path(project_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("read line {} from {}", index + 1, path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)
            .with_context(|| format!("parse line {} from {}", index + 1, path.display()))?;
        validate_event(&value).with_context(|| format!("validate line {} from {}", index + 1, path.display()))?;
        records.push(value);
    }

    let start = records.len().saturating_sub(limit);
    Ok(records.split_off(start))
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
}

impl RecoveryLock {
    fn acquire(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join(".recovery.lock");
        let started = Instant::now();
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path) {
                        match fs::remove_dir_all(&path) {
                            Ok(()) => continue,
                            Err(remove_error)
                                if remove_error.kind() == std::io::ErrorKind::NotFound =>
                            {
                                continue;
                            }
                            Err(remove_error) => {
                                return Err(remove_error).with_context(|| {
                                    format!("remove stale recovery lock {}", path.display())
                                });
                            }
                        }
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
}

impl Drop for RecoveryLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
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
        let decision = super::super::resolve_recovery(
            temp.path(),
            super::super::FAILURE_TEST,
            2,
            Some(super::super::PROFILE_DEEP),
            Some("compiler evidence".to_string()),
        )
        .expect("recovery decision");
        append(temp.path(), &decision).expect("append recovery event");
        let records = history(temp.path(), 10).expect("history");
        assert_eq!(records.len(), 1);
        let stored = &records[0]["decision"];
        assert_eq!(stored["failure_class"], "test");
        assert_eq!(stored["next_profile"], "critical");
        assert_eq!(stored["rollback_recommended"], true);
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
