from pathlib import Path

LEASE_PATH = Path("codex-rs/cli/src/bin/codexflow/lease.rs")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


text = LEASE_PATH.read_text(encoding="utf-8")
if "struct ScopeMutationLock" in text:
    print("Phase 3C lease mutation locking is already applied")
    raise SystemExit(0)

text = replace_once(
    text,
    "use std::thread;\nuse std::time::Duration;\n",
    "use std::thread;\nuse std::time::Duration;\nuse std::time::Instant;\n",
    "lease lock imports",
)

text = replace_once(
    text,
    "const METADATA_RETRY_COUNT: usize = 4;\nconst METADATA_RETRY_DELAY: Duration = Duration::from_millis(8);\n",
    "const METADATA_RETRY_COUNT: usize = 4;\nconst METADATA_RETRY_DELAY: Duration = Duration::from_millis(8);\nconst LOCK_RETRY_DELAY: Duration = Duration::from_millis(10);\nconst LOCK_WAIT_LIMIT: Duration = Duration::from_secs(5);\nconst LOCK_STALE_AFTER: Duration = Duration::from_secs(30);\n",
    "lease lock constants",
)

text = replace_once(
    text,
    "fn acquire(\n    project_root: &Path,\n    scope: &str,\n    owner: &str,\n    task: Option<&str>,\n    ttl_seconds: u64,\n) -> Result<LeaseRecord> {\n",
    "fn acquire(\n    project_root: &Path,\n    scope: &str,\n    owner: &str,\n    task: Option<&str>,\n    ttl_seconds: u64,\n) -> Result<LeaseRecord> {\n    validate_token(scope, \"lease scope\")?;\n    let base = lease_base_dir(project_root)?;\n    fs::create_dir_all(&base).with_context(|| format!(\"create {}\", base.display()))?;\n    let _guard = ScopeMutationLock::acquire(&base, scope)?;\n    acquire_unlocked(project_root, scope, owner, task, ttl_seconds)\n}\n\nfn acquire_unlocked(\n    project_root: &Path,\n    scope: &str,\n    owner: &str,\n    task: Option<&str>,\n    ttl_seconds: u64,\n) -> Result<LeaseRecord> {\n",
    "acquire wrapper",
)

text = replace_once(
    text,
    "return renew(project_root, scope, owner, ttl_seconds);",
    "return renew_unlocked(project_root, scope, owner, ttl_seconds);",
    "same-owner acquire renewal",
)

text = replace_once(
    text,
    "fn renew(\n    project_root: &Path,\n    scope: &str,\n    owner: &str,\n    ttl_seconds: u64,\n) -> Result<LeaseRecord> {\n",
    "fn renew(\n    project_root: &Path,\n    scope: &str,\n    owner: &str,\n    ttl_seconds: u64,\n) -> Result<LeaseRecord> {\n    validate_token(scope, \"lease scope\")?;\n    let base = lease_base_dir(project_root)?;\n    let _guard = ScopeMutationLock::acquire(&base, scope)?;\n    renew_unlocked(project_root, scope, owner, ttl_seconds)\n}\n\nfn renew_unlocked(\n    project_root: &Path,\n    scope: &str,\n    owner: &str,\n    ttl_seconds: u64,\n) -> Result<LeaseRecord> {\n",
    "renew wrapper",
)

text = replace_once(
    text,
    "fn release(project_root: &Path, scope: &str, owner: &str) -> Result<()> {\n",
    "fn release(project_root: &Path, scope: &str, owner: &str) -> Result<()> {\n    validate_token(scope, \"lease scope\")?;\n    let base = lease_base_dir(project_root)?;\n    let _guard = ScopeMutationLock::acquire(&base, scope)?;\n    release_unlocked(project_root, scope, owner)\n}\n\nfn release_unlocked(project_root: &Path, scope: &str, owner: &str) -> Result<()> {\n",
    "release wrapper",
)

text = replace_once(
    text,
    "        let scope_dir = entry.path();\n        if !scope_dir.is_dir() {\n            continue;\n        }\n        let lease = match load_record_retry(&scope_dir) {\n            Ok(lease) => lease,\n            Err(_) => continue,\n        };\n        if expired(&lease) {\n",
    "        let scope_dir = entry.path();\n        if !scope_dir.is_dir() {\n            continue;\n        }\n        let Some(scope_name) = scope_dir\n            .file_name()\n            .and_then(|value| value.to_str())\n            .map(str::to_string)\n        else {\n            continue;\n        };\n        if validate_token(&scope_name, \"lease scope\").is_err() {\n            continue;\n        }\n        let _guard = ScopeMutationLock::acquire(&base, &scope_name)?;\n        if !scope_dir.is_dir() {\n            continue;\n        }\n        let lease = match load_record_retry(&scope_dir) {\n            Ok(lease) => lease,\n            Err(_) => continue,\n        };\n        if lease.scope != scope_name {\n            continue;\n        }\n        if expired(&lease) {\n",
    "prune serialization",
)

text = replace_once(
    text,
    "fn lease_base_dir(project_root: &Path) -> Result<PathBuf> {\n",
    "struct ScopeMutationLock {\n    path: PathBuf,\n}\n\nimpl ScopeMutationLock {\n    fn acquire(base: &Path, scope: &str) -> Result<Self> {\n        let lock_root = base.join(\".locks\");\n        fs::create_dir_all(&lock_root)\n            .with_context(|| format!(\"create {}\", lock_root.display()))?;\n        let path = lock_root.join(format!(\"{scope}.lock\"));\n        let started = Instant::now();\n\n        loop {\n            match fs::create_dir(&path) {\n                Ok(()) => return Ok(Self { path }),\n                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {\n                    let stale = fs::metadata(&path)\n                        .and_then(|metadata| metadata.modified())\n                        .ok()\n                        .and_then(|modified| modified.elapsed().ok())\n                        .is_some_and(|elapsed| elapsed > LOCK_STALE_AFTER);\n                    if stale {\n                        match fs::remove_dir(&path) {\n                            Ok(()) => continue,\n                            Err(remove_error)\n                                if remove_error.kind() == std::io::ErrorKind::NotFound =>\n                            {\n                                continue;\n                            }\n                            Err(remove_error) => {\n                                return Err(remove_error).with_context(|| {\n                                    format!(\"remove stale lease lock {}\", path.display())\n                                });\n                            }\n                        }\n                    }\n                    if started.elapsed() > LOCK_WAIT_LIMIT {\n                        bail!(\"timed out waiting for lease mutation lock {}\", path.display());\n                    }\n                    thread::sleep(LOCK_RETRY_DELAY);\n                }\n                Err(error) => {\n                    return Err(error)\n                        .with_context(|| format!(\"create lease mutation lock {}\", path.display()));\n                }\n            }\n        }\n    }\n}\n\nimpl Drop for ScopeMutationLock {\n    fn drop(&mut self) {\n        let _ = fs::remove_dir(&self.path);\n    }\n}\n\nfn lease_base_dir(project_root: &Path) -> Result<PathBuf> {\n",
    "scope mutation lock implementation",
)

text = replace_once(
    text,
    "mod tests {\n    use super::*;\n",
    "mod tests {\n    use super::*;\n    use std::sync::mpsc;\n",
    "lease test imports",
)

text = replace_once(
    text,
    "    #[test]\n    fn task_scope_validates_task_id() {\n",
    "    #[test]\n    fn scope_mutation_lock_serializes_same_scope() {\n        let temp = tempfile::tempdir().expect(\"tempdir\");\n        init_repo(temp.path());\n        let base = lease_base_dir(temp.path()).expect(\"base\");\n        fs::create_dir_all(&base).expect(\"base dir\");\n        let first = ScopeMutationLock::acquire(&base, \"task-demo\").expect(\"first lock\");\n        let worker_base = base.clone();\n        let (ready_tx, ready_rx) = mpsc::channel();\n        let (acquired_tx, acquired_rx) = mpsc::channel();\n        let worker = std::thread::spawn(move || {\n            ready_tx.send(()).expect(\"signal ready\");\n            let _second =\n                ScopeMutationLock::acquire(&worker_base, \"task-demo\").expect(\"second lock\");\n            acquired_tx.send(()).expect(\"signal acquired\");\n        });\n\n        ready_rx\n            .recv_timeout(std::time::Duration::from_secs(1))\n            .expect(\"worker ready\");\n        assert!(\n            acquired_rx\n                .recv_timeout(std::time::Duration::from_millis(50))\n                .is_err()\n        );\n        drop(first);\n        acquired_rx\n            .recv_timeout(std::time::Duration::from_secs(1))\n            .expect(\"worker acquired after release\");\n        worker.join().expect(\"worker join\");\n    }\n\n    #[test]\n    fn previous_owner_cannot_release_reacquired_scope() {\n        let temp = tempfile::tempdir().expect(\"tempdir\");\n        init_repo(temp.path());\n        let base = lease_base_dir(temp.path()).expect(\"base\");\n        let scope = scope_dir(&base, \"task-demo\");\n        fs::create_dir_all(&scope).expect(\"scope dir\");\n        let expired_record = LeaseRecord {\n            schema: LEASE_SCHEMA.to_string(),\n            scope: \"task-demo\".to_string(),\n            owner: \"worker-a\".to_string(),\n            task: Some(\"demo\".to_string()),\n            acquired_at: \"2020-01-01T00:00:00Z\".to_string(),\n            renewed_at: \"2020-01-01T00:00:00Z\".to_string(),\n            expires_at_ms: 1,\n        };\n        atomic_write_record(&record_path(&scope), &expired_record).expect(\"expired record\");\n        let current = acquire(temp.path(), \"task-demo\", \"worker-b\", Some(\"demo\"), 300)\n            .expect(\"reacquire\");\n        assert_eq!(current.owner, \"worker-b\");\n        let error = release(temp.path(), \"task-demo\", \"worker-a\")\n            .expect_err(\"previous owner must not release current lease\");\n        assert!(error.to_string().contains(\"owned by worker-b\"));\n        let persisted = load_record_retry(&scope).expect(\"current record\");\n        assert_eq!(persisted.owner, \"worker-b\");\n    }\n\n    #[test]\n    fn task_scope_validates_task_id() {\n",
    "lease concurrency tests",
)

LEASE_PATH.write_text(text, encoding="utf-8")
print("Applied Phase 3C lease mutation locking")
