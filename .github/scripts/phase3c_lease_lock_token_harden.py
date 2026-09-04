from pathlib import Path

LEASE_PATH = Path("codex-rs/cli/src/bin/codexflow/lease.rs")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


text = LEASE_PATH.read_text(encoding="utf-8")
if "LOCK_TOKEN_COUNTER" in text:
    print("Phase 3C token-guarded lease locking is already applied")
    raise SystemExit(0)

text = replace_once(
    text,
    "use std::fs;\n",
    "use std::fs;\nuse std::fs::OpenOptions;\nuse std::io::Write;\nuse std::sync::atomic::AtomicU64;\nuse std::sync::atomic::Ordering;\n",
    "token lock imports",
)

text = replace_once(
    text,
    "const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);\n",
    "const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);\nstatic LOCK_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);\n",
    "token lock counter",
)

old_lock = '''struct ScopeMutationLock {
    path: PathBuf,
}

impl ScopeMutationLock {
    fn acquire(base: &Path, scope: &str) -> Result<Self> {
        let lock_root = base.join(".locks");
        fs::create_dir_all(&lock_root)
            .with_context(|| format!("create {}", lock_root.display()))?;
        let path = lock_root.join(format!("{scope}.lock"));
        let started = Instant::now();

        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|elapsed| elapsed > LOCK_STALE_AFTER);
                    if stale {
                        match fs::remove_dir(&path) {
                            Ok(()) => continue,
                            Err(remove_error)
                                if remove_error.kind() == std::io::ErrorKind::NotFound =>
                            {
                                continue;
                            }
                            Err(remove_error) => {
                                return Err(remove_error).with_context(|| {
                                    format!("remove stale lease lock {}", path.display())
                                });
                            }
                        }
                    }
                    if started.elapsed() > LOCK_WAIT_LIMIT {
                        bail!("timed out waiting for lease mutation lock {}", path.display());
                    }
                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("create lease mutation lock {}", path.display()));
                }
            }
        }
    }
}

impl Drop for ScopeMutationLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}
'''

new_lock = '''struct ScopeMutationLock {
    path: PathBuf,
    token: String,
}

impl ScopeMutationLock {
    fn acquire(base: &Path, scope: &str) -> Result<Self> {
        let lock_root = base.join(".locks");
        fs::create_dir_all(&lock_root)
            .with_context(|| format!("create {}", lock_root.display()))?;
        let path = lock_root.join(format!("{scope}.lock"));
        let token = format!(
            "{}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros(),
            LOCK_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let started = Instant::now();

        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let write_result = file
                        .write_all(token.as_bytes())
                        .and_then(|_| file.sync_all());
                    if let Err(error) = write_result {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error).with_context(|| {
                            format!("initialize lease mutation lock {}", path.display())
                        });
                    }
                    return Ok(Self {
                        path,
                        token: token.clone(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|elapsed| elapsed > LOCK_STALE_AFTER);
                    if stale {
                        bail!(
                            "stale lease mutation lock {}; refusing automatic reap to preserve ownership safety",
                            path.display()
                        );
                    }
                    if started.elapsed() > LOCK_WAIT_LIMIT {
                        bail!("timed out waiting for lease mutation lock {}", path.display());
                    }
                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("create lease mutation lock {}", path.display()));
                }
            }
        }
    }
}

impl Drop for ScopeMutationLock {
    fn drop(&mut self) {
        let still_owned = fs::read_to_string(&self.path)
            .is_ok_and(|token| token == self.token);
        if still_owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}
'''

text = replace_once(text, old_lock, new_lock, "token-guarded scope lock")

text = replace_once(
    text,
    "    #[test]\n    fn previous_owner_cannot_release_reacquired_scope() {\n",
    '''    #[test]
    fn scope_mutation_lock_drop_preserves_replacement_token() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_repo(temp.path());
        let base = lease_base_dir(temp.path()).expect("base");
        fs::create_dir_all(&base).expect("base dir");
        let first = ScopeMutationLock::acquire(&base, "task-demo").expect("first lock");
        let path = first.path.clone();
        fs::remove_file(&path).expect("simulate replacement");
        fs::write(&path, "replacement-token").expect("write replacement token");
        drop(first);
        assert_eq!(
            fs::read_to_string(&path).expect("replacement survives"),
            "replacement-token"
        );
    }

    #[test]
    fn previous_owner_cannot_release_reacquired_scope() {
''',
    "token ownership regression test",
)

LEASE_PATH.write_text(text, encoding="utf-8")
print("Applied Phase 3C token-guarded lease locking")
