from pathlib import Path

LEASE_PATH = Path("codex-rs/cli/src/bin/codexflow/lease.rs")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


text = LEASE_PATH.read_text(encoding="utf-8")
if "use std::fs::TryLockError;" in text:
    print("Phase 3C OS-backed lease locking is already applied")
    raise SystemExit(0)

text = replace_once(
    text,
    "use std::fs;\n",
    "use std::fs;\nuse std::fs::File;\nuse std::fs::OpenOptions;\nuse std::fs::TryLockError;\n",
    "OS lock imports",
)

text = replace_once(
    text,
    "const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);\n",
    "",
    "remove stale lock timeout",
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
                        bail!("timed out waiting for lease mutation lock {}", path.display());
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
'''

text = replace_once(text, old_lock, new_lock, "OS-backed scope lock")

LEASE_PATH.write_text(text, encoding="utf-8")
print("Applied Phase 3C OS-backed lease locking")
