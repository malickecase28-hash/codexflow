from pathlib import Path

SUPERVISOR_PATH = Path("codex-rs/cli/src/bin/codexflow-supervisor.rs")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


text = SUPERVISOR_PATH.read_text(encoding="utf-8")
if "use std::fs::TryLockError;" in text:
    print("Phase 3C OS-backed supervisor locking is already applied")
    raise SystemExit(0)

text = replace_once(
    text,
    "use std::fs;\nuse std::fs::OpenOptions;\n",
    "use std::fs;\nuse std::fs::File;\nuse std::fs::OpenOptions;\nuse std::fs::TryLockError;\n",
    "supervisor lock imports",
)
text = replace_once(
    text,
    "const LOCK_STALE_AFTER: Duration = Duration::from_secs(120);\n",
    "",
    "remove supervisor stale timeout",
)

old_lock = '''struct StateLock {
    path: PathBuf,
}

impl StateLock {
    fn acquire(project_root: &Path) -> Result<Self> {
        ensure_state_dirs(project_root)?;
        let path = state_dir(project_root).join(".lock");
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "pid={}", std::process::id())?;
                    return Ok(Self { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|elapsed| elapsed > LOCK_STALE_AFTER);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > LOCK_WAIT_LIMIT {
                        bail!(
                            "timed out waiting for supervisor state lock {}",
                            path.display()
                        );
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(err) => return Err(err).context("create supervisor state lock"),
            }
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
'''

new_lock = '''struct StateLock {
    _file: File,
}

impl StateLock {
    fn acquire(project_root: &Path) -> Result<Self> {
        ensure_state_dirs(project_root)?;
        let path = state_dir(project_root).join(".lock");
        if path.is_dir() {
            bail!(
                "legacy supervisor state lock directory {}; remove it after confirming no older CodexFlow process is active",
                path.display()
            );
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("open supervisor state lock {}", path.display()))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => {
                    file.set_len(0)
                        .with_context(|| format!("truncate supervisor state lock {}", path.display()))?;
                    writeln!(file, "pid={} acquired_at={}", std::process::id(), now_iso())
                        .with_context(|| format!("write supervisor state lock {}", path.display()))?;
                    file.sync_data()
                        .with_context(|| format!("sync supervisor state lock {}", path.display()))?;
                    return Ok(Self { _file: file });
                }
                Err(TryLockError::WouldBlock) => {
                    if started.elapsed() > LOCK_WAIT_LIMIT {
                        bail!(
                            "timed out waiting for supervisor state lock {}",
                            path.display()
                        );
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error)
                        .with_context(|| format!("lock supervisor state {}", path.display()));
                }
            }
        }
    }
}
'''
text = replace_once(text, old_lock, new_lock, "OS-backed supervisor state lock")

marker = "    #[test]\n    fn wildcard_topic_matches_expected_event_kinds() {"
new_test = '''    #[test]
    fn state_lock_releases_when_handle_drops() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        let guard = StateLock::acquire(root).expect("state lock");
        let path = state_dir(root).join(".lock");
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("second state lock handle");
        assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
        drop(guard);
        second.try_lock().expect("state lock released after drop");
    }

'''
text = replace_once(text, marker, new_test + marker, "supervisor state lock regression test")

SUPERVISOR_PATH.write_text(text, encoding="utf-8")
print("Applied Phase 3C OS-backed supervisor state locking")
