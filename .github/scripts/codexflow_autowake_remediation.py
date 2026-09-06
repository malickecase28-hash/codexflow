from pathlib import Path

runtime_path = Path("codex-rs/cli/src/bin/codexflow/runtime.rs")
runtime = runtime_path.read_text()

old_wait = '''            append_event(
                project_root,
                "task.waiting",
                "runtime",
                Some(&id),
                &format!("waiting on {await_id}"),
            )?;
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskWake'''
new_wait = '''            append_event(
                project_root,
                "task.waiting",
                "runtime",
                Some(&id),
                &format!("waiting on {await_id}"),
            )?;
            // Close the race where the durable supervisor fires an await before
            // the workflow has persisted its BLOCKED_WAITING transition.
            if supervisor_await_fired(project_root, &await_id)? {
                task.status = "doing".to_string();
                task.waiting_on = None;
                task.updated_at = now_iso();
                append_event(
                    project_root,
                    "task.woke",
                    "runtime",
                    Some(&id),
                    &format!("await {await_id} already fired"),
                )?;
            }
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
        RuntimeCommand::TaskWake'''
if runtime.count(old_wait) != 1:
    raise SystemExit(f"expected one TaskWait integration seam, found {runtime.count(old_wait)}")
runtime = runtime.replace(old_wait, new_wait)

old_paths = '''fn event_path(project_root: &Path) -> PathBuf {
    runtime_dir(project_root).join("events-v2.jsonl")
}

fn now_iso() -> String {'''
new_paths = '''fn event_path(project_root: &Path) -> PathBuf {
    runtime_dir(project_root).join("events-v2.jsonl")
}

fn supervisor_awaits_path(project_root: &Path) -> PathBuf {
    runtime_dir(project_root)
        .join("supervisor")
        .join("awaits.json")
}

fn supervisor_await_fired(project_root: &Path, await_id: &str) -> Result<bool> {
    let path = supervisor_awaits_path(project_root);
    if !path.exists() {
        return Ok(false);
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let awaits: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    Ok(awaits
        .get(await_id)
        .and_then(|spec| spec.get("state"))
        .and_then(serde_json::Value::as_str)
        == Some("fired"))
}

fn now_iso() -> String {'''
if runtime.count(old_paths) != 1:
    raise SystemExit(f"expected one runtime path seam, found {runtime.count(old_paths)}")
runtime = runtime.replace(old_paths, new_paths)

runtime_test = r'''

    #[test]
    fn task_wait_does_not_block_when_supervisor_await_already_fired() {
        let temp = tempfile::tempdir().expect("create runtime state");
        let root = temp.path();
        fs::create_dir_all(
            root.join(".codexflow")
                .join("state")
                .join("supervisor"),
        )
        .expect("create supervisor state");
        fs::write(
            supervisor_awaits_path(root),
            serde_json::to_vec_pretty(&serde_json::json!({
                "build_wait": { "state": "fired" }
            }))
            .expect("serialize awaits"),
        )
        .expect("write awaits");

        let mut ledger = default_ledger(root);
        ledger.tasks.insert(
            "task-a".to_string(),
            TaskRecord {
                title: "waiting task".to_string(),
                status: "doing".to_string(),
                risk: "medium".to_string(),
                assignee: None,
                depends_on: Vec::new(),
                budget_tokens: None,
                used_tokens: 0,
                waiting_on: None,
                acceptance: Vec::new(),
                gates: BTreeMap::new(),
                handoffs: Vec::new(),
                updated_at: now_iso(),
            },
        );
        save_ledger(root, &ledger).expect("seed runtime ledger");

        handle(
            root,
            RuntimeArgs {
                project: None,
                command: RuntimeCommand::TaskWait {
                    id: "task-a".to_string(),
                    await_id: "build_wait".to_string(),
                },
            },
        )
        .expect("wait on already fired await");

        let ledger = load_or_init(root).expect("reload runtime ledger");
        let task = ledger.tasks.get("task-a").expect("task-a");
        assert_eq!(task.status, "doing");
        assert_eq!(task.waiting_on, None);
        let events = fs::read_to_string(event_path(root)).expect("read runtime events");
        assert_eq!(events.matches("\"kind\":\"task.woke\"").count(), 1);
    }
'''
if "task_wait_does_not_block_when_supervisor_await_already_fired" in runtime:
    raise SystemExit("runtime auto-wake test already present")
idx = runtime.rfind("\n}")
if idx < 0:
    raise SystemExit("could not find runtime test module terminator")
runtime = runtime[:idx] + runtime_test + runtime[idx:]
runtime_path.write_text(runtime)

supervisor_path = Path("codex-rs/cli/src/bin/codexflow-supervisor.rs")
supervisor = supervisor_path.read_text()

old_start = '''    ensure_state_dirs(project_root)?;
    reconcile_waiting_awaits(project_root)?;
    process_due_timeouts(project_root)?;
    let bind_address = parse_loopback_address(bind)?;'''
new_start = '''    ensure_state_dirs(project_root)?;
    reconcile_waiting_awaits(project_root)?;
    process_due_timeouts(project_root)?;
    // A prior process can crash after persisting a fired await but before the
    // workflow ledger is resumed. Reconcile that durable boundary at startup.
    reconcile_fired_runtime_wakes(project_root)?;
    let bind_address = parse_loopback_address(bind)?;'''
if supervisor.count(old_start) != 1:
    raise SystemExit(f"expected one supervisor startup seam, found {supervisor.count(old_start)}")
supervisor = supervisor.replace(old_start, new_start)

old_fire = '''    let record = InboxRecord {
        schema: INBOX_SCHEMA.to_string(),
        await_id: spec.id.clone(),
        event: event.clone(),
        delivered_at: now_iso(),
    };
    append_inbox_once(project_root, &spec.owner, &record)
}'''
new_fire = '''    let owner = spec.owner.clone();
    let record = InboxRecord {
        schema: INBOX_SCHEMA.to_string(),
        await_id: spec.id.clone(),
        event: event.clone(),
        delivered_at: now_iso(),
    };
    append_inbox_once(project_root, &owner, &record)?;
    // Persist the fired state before waking the runtime. This ordering closes
    // the race where TaskWait starts after a no-op wake but before awaits.json
    // reflects the fired event.
    save_awaits(project_root, awaits)?;
    wake_runtime_tasks_for_await(project_root, await_id)?;
    Ok(())
}'''
if supervisor.count(old_fire) != 1:
    raise SystemExit(f"expected one fire_await seam, found {supervisor.count(old_fire)}")
supervisor = supervisor.replace(old_fire, new_fire)

lock_seam = '''fn with_state_lock<T>(project_root: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = StateLock::acquire(project_root)?;
    f()
}'''
bridge = r'''fn runtime_state_dir(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("state")
}

fn runtime_ledger_path(project_root: &Path) -> PathBuf {
    runtime_state_dir(project_root).join("runtime-v2.json")
}

fn runtime_event_path(project_root: &Path) -> PathBuf {
    runtime_state_dir(project_root).join("events-v2.jsonl")
}

fn wake_runtime_tasks_for_await(project_root: &Path, await_id: &str) -> Result<usize> {
    let path = runtime_ledger_path(project_root);
    if !path.exists() {
        return Ok(0);
    }
    let _guard = RuntimeLedgerLock::acquire(project_root)?;
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut ledger: Value =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    if ledger.get("schema").and_then(Value::as_str) != Some("codexflow.runtime.v2") {
        bail!("unsupported CodexFlow runtime ledger schema while waking await {await_id}");
    }

    let tasks = ledger
        .get_mut("tasks")
        .and_then(Value::as_object_mut)
        .context("CodexFlow runtime ledger is missing tasks object")?;
    let mut woke = Vec::new();
    for (task_id, task_value) in tasks {
        let Some(task) = task_value.as_object_mut() else {
            bail!("CodexFlow runtime task {task_id} is not an object");
        };
        let blocked = task.get("status").and_then(Value::as_str) == Some("blocked_waiting");
        let waiting_on = task.get("waiting_on").and_then(Value::as_str) == Some(await_id);
        if blocked && waiting_on {
            task.insert("status".to_string(), Value::String("doing".to_string()));
            task.insert("waiting_on".to_string(), Value::Null);
            task.insert("updated_at".to_string(), Value::String(now_iso()));
            woke.push(task_id.clone());
        }
    }
    if woke.is_empty() {
        return Ok(0);
    }

    if let Some(object) = ledger.as_object_mut() {
        object.insert("updated_at".to_string(), Value::String(now_iso()));
    }
    write_json_atomic(&path, &ledger)?;
    for task_id in &woke {
        append_json_line(
            &runtime_event_path(project_root),
            &json!({
                "ts": now_iso(),
                "kind": "task.woke",
                "actor": "supervisor",
                "task": task_id,
                "message": format!("await {await_id} fired")
            }),
        )?;
    }
    Ok(woke.len())
}

fn reconcile_fired_runtime_wakes(project_root: &Path) -> Result<usize> {
    let fired_ids = with_state_lock(project_root, || {
        let awaits = load_awaits(project_root)?;
        Ok(awaits
            .values()
            .filter(|spec| spec.state == "fired")
            .map(|spec| spec.id.clone())
            .collect::<Vec<_>>())
    })?;
    let mut woke = 0usize;
    for await_id in fired_ids {
        woke = woke.saturating_add(wake_runtime_tasks_for_await(project_root, &await_id)?);
    }
    Ok(woke)
}

struct RuntimeLedgerLock {
    path: PathBuf,
}

impl RuntimeLedgerLock {
    fn acquire(project_root: &Path) -> Result<Self> {
        fs::create_dir_all(runtime_state_dir(project_root))?;
        let path = runtime_state_dir(project_root).join(".runtime.lock");
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "pid={}", std::process::id())?;
                    file.sync_data()
                        .with_context(|| format!("sync runtime lock {}", path.display()))?;
                    return Ok(Self { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|elapsed| elapsed > Duration::from_secs(120));
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > LOCK_WAIT_LIMIT {
                        bail!("timed out waiting for runtime lock {}", path.display());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(err).context("create runtime lock"),
            }
        }
    }
}

impl Drop for RuntimeLedgerLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

'''
if supervisor.count(lock_seam) != 1:
    raise SystemExit(f"expected one supervisor state-lock seam, found {supervisor.count(lock_seam)}")
supervisor = supervisor.replace(lock_seam, bridge + lock_seam)

supervisor_test = r'''

    fn seed_blocked_runtime_task(root: &Path, task_id: &str, await_id: &str) {
        let mut tasks = serde_json::Map::new();
        tasks.insert(
            task_id.to_string(),
            json!({
                "title": "blocked task",
                "status": "blocked_waiting",
                "risk": "medium",
                "assignee": null,
                "depends_on": [],
                "budget_tokens": null,
                "used_tokens": 0,
                "waiting_on": await_id,
                "acceptance": [],
                "gates": {},
                "handoffs": [],
                "updated_at": "old"
            }),
        );
        write_json_atomic(
            &runtime_ledger_path(root),
            &json!({
                "schema": "codexflow.runtime.v2",
                "project_root": root,
                "created_at": "old",
                "updated_at": "old",
                "tasks": Value::Object(tasks),
                "agents": {}
            }),
        )
        .expect("seed runtime ledger");
    }

    #[test]
    fn fired_await_resumes_blocked_runtime_task_exactly_once() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        seed_blocked_runtime_task(root, "task-a", "build_wait");
        register_await(
            root,
            "build_wait".to_string(),
            "god".to_string(),
            vec!["build.completed".to_string()],
            Some("job-1".to_string()),
            0,
        )
        .expect("register await");
        publish_event_with_dedupe(
            root,
            "build.completed".to_string(),
            Some("job-1".to_string()),
            Some("build:job-1".to_string()),
            json!({"status":"ok"}),
        )
        .expect("publish wake event");

        let ledger: Value = serde_json::from_str(
            &fs::read_to_string(runtime_ledger_path(root)).expect("read runtime ledger"),
        )
        .expect("parse runtime ledger");
        assert_eq!(ledger["tasks"]["task-a"]["status"], "doing");
        assert!(ledger["tasks"]["task-a"]["waiting_on"].is_null());
        assert_eq!(reconcile_fired_runtime_wakes(root).expect("reconcile fired waits"), 0);
        let events: Vec<Value> =
            read_json_lines(&runtime_event_path(root)).expect("read runtime events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["kind"] == "task.woke")
                .count(),
            1
        );
    }

    #[test]
    fn startup_reconciliation_heals_crash_after_await_persisted_fired() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        register_await(
            root,
            "build_wait".to_string(),
            "god".to_string(),
            vec!["build.completed".to_string()],
            None,
            0,
        )
        .expect("register await");
        publish_event_with_dedupe(
            root,
            "build.completed".to_string(),
            None,
            Some("crash-boundary".to_string()),
            json!({"status":"ok"}),
        )
        .expect("persist fired await");

        seed_blocked_runtime_task(root, "task-a", "build_wait");
        assert_eq!(reconcile_fired_runtime_wakes(root).expect("heal wake"), 1);
        assert_eq!(reconcile_fired_runtime_wakes(root).expect("repeat heal"), 0);
        let ledger: Value = serde_json::from_str(
            &fs::read_to_string(runtime_ledger_path(root)).expect("read runtime ledger"),
        )
        .expect("parse runtime ledger");
        assert_eq!(ledger["tasks"]["task-a"]["status"], "doing");
    }
'''
if "fired_await_resumes_blocked_runtime_task_exactly_once" in supervisor:
    raise SystemExit("supervisor auto-wake tests already present")
idx = supervisor.rfind("\n}")
if idx < 0:
    raise SystemExit("could not find supervisor test module terminator")
supervisor = supervisor[:idx] + supervisor_test + supervisor[idx:]
supervisor_path.write_text(supervisor)
