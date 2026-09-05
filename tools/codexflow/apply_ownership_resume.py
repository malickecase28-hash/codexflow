#!/usr/bin/env python3
"""Materialize the ownership/resume tranche from the verified CodexFlow baseline.

This is intentionally assertion-heavy. It refuses to patch if the expected baseline
shape changed, which makes it safe to use for an isolated validation branch and
prevents fuzzy text edits from silently corrupting runtime semantics.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "codex-rs/cli/src/bin/codexflow.rs"
RUNTIME = ROOT / "codex-rs/cli/src/bin/codexflow/runtime.rs"
LEASE = ROOT / "codex-rs/cli/src/bin/codexflow/lease.rs"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one baseline match, found {count}")
    return text.replace(old, new, 1)


def replace_between(text: str, start: str, end: str, replacement: str, label: str) -> str:
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"{label}: start marker not found")
    end_index = text.find(end, start_index + len(start))
    if end_index < 0:
        raise SystemExit(f"{label}: end marker not found")
    return text[:start_index] + replacement + text[end_index:]


def patch_cli() -> None:
    text = CLI.read_text()
    text = replace_once(
        text,
        '#[path = "codexflow/routing.rs"]\nmod routing;\n#[path = "codexflow/runtime.rs"]\nmod runtime_state;\n',
        '#[path = "codexflow/routing.rs"]\nmod routing;\n#[path = "codexflow/resume.rs"]\nmod resume;\n#[path = "codexflow/runtime.rs"]\nmod runtime_state;\n',
        "cli resume module",
    )
    text = replace_once(
        text,
        '    Orchestrate(orchestrator::OrchestrateArgs),\n    Route(routing::RoutingArgs),\n    Delivery(delivery::DeliveryArgs),\n',
        '    Orchestrate(orchestrator::OrchestrateArgs),\n    Route(routing::RoutingArgs),\n    Resume(resume::ResumeArgs),\n    Delivery(delivery::DeliveryArgs),\n',
        "cli resume command",
    )
    text = replace_once(
        text,
        '                Some(TopCommand::Route(args)) => {\n                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;\n                    routing::handle(&primary_root(&project)?, args)\n                }\n                Some(TopCommand::Delivery(args)) => {\n',
        '                Some(TopCommand::Route(args)) => {\n                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;\n                    routing::handle(&primary_root(&project)?, args)\n                }\n                Some(TopCommand::Resume(args)) => {\n                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;\n                    resume::handle(&primary_root(&project)?, args)\n                }\n                Some(TopCommand::Delivery(args)) => {\n',
        "cli resume handler",
    )
    CLI.write_text(text)


def patch_runtime() -> None:
    text = RUNTIME.read_text()
    text = replace_once(
        text,
        'use std::time::Duration;\nuse std::time::Instant;\n\nconst SCHEMA: &str = "codexflow.runtime.v2";\n',
        'use std::time::Duration;\nuse std::time::Instant;\n\n#[path = "lease.rs"]\nmod lease;\n\nconst SCHEMA: &str = "codexflow.runtime.v2";\n',
        "runtime lease module",
    )

    create_start = '        RuntimeCommand::TaskCreate {\n'
    set_start = '        RuntimeCommand::TaskSet {\n'
    start_index = text.find(create_start)
    end_index = text.find(set_start, start_index)
    if start_index < 0 or end_index < 0:
        raise SystemExit("runtime task-create segment markers missing")
    segment = text[start_index:end_index]
    segment = replace_once(
        segment,
        '            validate_risk(&risk)?;\n',
        '            validate_risk(&risk)?;\n            if assignee.is_some() {\n                bail!("task-create --assignee is disabled; assign ownership with runtime agent-set");\n            }\n',
        "runtime task-create assignee guard",
    )
    segment = replace_once(
        segment,
        '                    assignee,\n',
        '                    assignee: None,\n',
        "runtime task-create assignee storage",
    )
    text = text[:start_index] + segment + text[end_index:]

    task_set = '''        RuntimeCommand::TaskSet {
            id,
            status,
            risk,
            assignee,
            budget_tokens,
        } => {
            if assignee.is_some() {
                bail!("task-set --assignee is disabled; assign ownership with runtime agent-set");
            }
            let mut release_owner = None;
            with_locked_ledger(project_root, |ledger| {
                let task = ledger
                    .tasks
                    .get_mut(&id)
                    .with_context(|| format!("unknown task: {id}"))?;
                if let Some(status) = status {
                    validate_task_status(&status)?;
                    if status == "blocked_waiting" {
                        bail!("use task-wait so blocked_waiting records the await id");
                    }
                    if status == "done" {
                        bail!("use task-complete so completion criteria and gates are checked");
                    }
                    if matches!(status.as_str(), "failed" | "cancelled") {
                        release_owner = task.assignee.take();
                    }
                    task.status = status;
                    task.waiting_on = None;
                }
                if let Some(risk) = risk {
                    validate_risk(&risk)?;
                    task.risk = risk;
                }
                if budget_tokens.is_some() {
                    task.budget_tokens = budget_tokens;
                }
                task.updated_at = now_iso();
                println!("{}", serde_json::to_string_pretty(task)?);
                Ok(())
            })?;
            if let Some(owner) = release_owner {
                lease::release_task_if_owned(project_root, &id, &owner)
                    .with_context(|| format!("task {id} entered terminal state but its ownership lease could not be released"))?;
            }
            Ok(())
        },
'''
    text = replace_between(text, set_start, '        RuntimeCommand::TaskAcceptanceAdd {\n', task_set, "runtime task-set")

    task_complete = '''        RuntimeCommand::TaskComplete { id, actor } => {
            validate_id(&actor)?;
            let owner = with_locked_ledger(project_root, |ledger| {
                let blockers = completion_blockers(ledger, &id)?;
                let check = CompletionCheck {
                    task: id.clone(),
                    ready: blockers.is_empty(),
                    blockers,
                };
                if !check.ready {
                    println!("{}", serde_json::to_string_pretty(&check)?);
                    bail!("task {id} is not ready for completion");
                }
                let task = ledger
                    .tasks
                    .get_mut(&id)
                    .with_context(|| format!("unknown task: {id}"))?;
                let owner = task.assignee.take();
                task.status = "done".to_string();
                task.waiting_on = None;
                task.updated_at = now_iso();
                append_event(
                    project_root,
                    "task.completed",
                    &actor,
                    Some(&id),
                    "completion gate passed",
                )?;
                println!("{}", serde_json::to_string_pretty(task)?);
                Ok(owner)
            })?;
            if let Some(owner) = owner {
                lease::release_task_if_owned(project_root, &id, &owner)
                    .with_context(|| format!("task {id} completed but its ownership lease could not be released"))?;
            }
            Ok(())
        },
'''
    text = replace_between(text, '        RuntimeCommand::TaskComplete { id, actor } =>', '        RuntimeCommand::TaskWait { id, await_id } =>', task_complete, "runtime task-complete")

    agent_set = '''        RuntimeCommand::AgentSet {
            name,
            role,
            status,
            task,
            budget_tokens,
        } => with_locked_ledger(project_root, |ledger| {
            validate_id(&name)?;
            validate_agent_status(&status)?;
            if let Some(task_id) = task.as_deref()
                && !ledger.tasks.contains_key(task_id)
            {
                bail!("unknown task: {task_id}");
            }

            let current = ledger.agents.get(&name).cloned();
            let previous_task = current.as_ref().and_then(|record| record.task.clone());
            let holds_lease = agent_status_holds_task_lease(&status) && task.is_some();
            let new_task = task.as_deref();

            if holds_lease
                && let Some(task_id) = new_task
            {
                lease::acquire_task(
                    project_root,
                    task_id,
                    &name,
                    lease::DEFAULT_TASK_LEASE_TTL_SECONDS,
                )?;
            }

            if let Some(previous_task_id) = previous_task.as_deref()
                && (Some(previous_task_id) != new_task || !holds_lease)
                && let Err(error) =
                    lease::release_task_if_owned(project_root, previous_task_id, &name)
            {
                if holds_lease
                    && let Some(task_id) = new_task
                    && task_id != previous_task_id
                {
                    let _ = lease::release_task_if_owned(project_root, task_id, &name);
                }
                return Err(error).with_context(|| {
                    format!("release previous task ownership {previous_task_id} for {name}")
                });
            }

            if let Some(previous_task_id) = previous_task.as_deref()
                && (Some(previous_task_id) != new_task || !holds_lease)
                && let Some(record) = ledger.tasks.get_mut(previous_task_id)
                && record.assignee.as_deref() == Some(name.as_str())
            {
                record.assignee = None;
                record.updated_at = now_iso();
            }

            if holds_lease
                && let Some(task_id) = new_task
            {
                let record = ledger.tasks.get_mut(task_id).expect("task validated above");
                record.assignee = Some(name.clone());
                record.updated_at = now_iso();
            } else if let Some(task_id) = new_task
                && let Some(record) = ledger.tasks.get_mut(task_id)
                && record.assignee.as_deref() == Some(name.as_str())
            {
                record.assignee = None;
                record.updated_at = now_iso();
            }

            let now = now_iso();
            ledger.agents.insert(
                name.clone(),
                AgentRecord {
                    role,
                    status,
                    task,
                    budget_tokens: budget_tokens.or(current.as_ref().and_then(|v| v.budget_tokens)),
                    used_tokens: current.as_ref().map_or(0, |v| v.used_tokens),
                    last_heartbeat_ms: Utc::now().timestamp_millis(),
                    last_progress: current.as_ref().and_then(|v| v.last_progress.clone()),
                    no_progress_count: current.as_ref().map_or(0, |v| v.no_progress_count),
                    last_action: current.as_ref().and_then(|v| v.last_action.clone()),
                    repeated_action_count: current.as_ref().map_or(0, |v| v.repeated_action_count),
                    consecutive_errors: current.as_ref().map_or(0, |v| v.consecutive_errors),
                    updated_at: now,
                },
            );
            append_event(
                project_root,
                "agent.updated",
                &name,
                new_task,
                if holds_lease {
                    "agent state updated with task ownership lease"
                } else {
                    "agent state updated without task ownership"
                },
            )?;
            println!("{name}");
            Ok(())
        }),
'''
    text = replace_between(text, '        RuntimeCommand::AgentSet {\n', '        RuntimeCommand::AgentHeartbeat { name, progress } =>', agent_set, "runtime agent-set")

    heartbeat = '''        RuntimeCommand::AgentHeartbeat { name, progress } => {
            with_locked_ledger(project_root, |ledger| {
                let (task_id, status) = {
                    let agent = ledger
                        .agents
                        .get(&name)
                        .with_context(|| format!("unknown agent: {name}"))?;
                    (agent.task.clone(), agent.status.clone())
                };
                if agent_status_holds_task_lease(&status)
                    && let Some(task_id) = task_id.as_deref()
                {
                    lease::renew_task(
                        project_root,
                        task_id,
                        &name,
                        lease::DEFAULT_TASK_LEASE_TTL_SECONDS,
                    )?;
                }
                let agent = ledger.agents.get_mut(&name).expect("agent validated above");
                if progress.is_some() && progress == agent.last_progress {
                    agent.no_progress_count = agent.no_progress_count.saturating_add(1);
                } else if progress.is_some() {
                    agent.no_progress_count = 0;
                }
                if progress.is_some() {
                    agent.last_progress = progress;
                }
                agent.last_heartbeat_ms = Utc::now().timestamp_millis();
                agent.updated_at = now_iso();
                println!("{}", serde_json::to_string_pretty(agent)?);
                Ok(())
            })
        }
'''
    text = replace_between(text, '        RuntimeCommand::AgentHeartbeat { name, progress } =>', '        RuntimeCommand::AgentAction {\n', heartbeat, "runtime heartbeat")

    agent_tokens = '''        RuntimeCommand::AgentTokens { name, add } => with_locked_ledger(project_root, |ledger| {
            let (task_id, status) = {
                let agent = ledger
                    .agents
                    .get(&name)
                    .with_context(|| format!("unknown agent: {name}"))?;
                (agent.task.clone(), agent.status.clone())
            };
            if agent_status_holds_task_lease(&status)
                && let Some(task_id) = task_id.as_deref()
            {
                lease::renew_task(
                    project_root,
                    task_id,
                    &name,
                    lease::DEFAULT_TASK_LEASE_TTL_SECONDS,
                )?;
            }
            let agent = ledger.agents.get_mut(&name).expect("agent validated above");
            agent.used_tokens = agent.used_tokens.saturating_add(add);
            if let Some(task_id) = task_id
                && let Some(task) = ledger.tasks.get_mut(&task_id)
            {
                task.used_tokens = task.used_tokens.saturating_add(add);
                task.updated_at = now_iso();
            }
            agent.updated_at = now_iso();
            println!("{}", serde_json::to_string_pretty(agent)?);
            Ok(())
        }),
'''
    text = replace_between(text, '        RuntimeCommand::AgentTokens { name, add } =>', '        RuntimeCommand::AgentList => {\n', agent_tokens, "runtime agent tokens")

    text = replace_once(
        text,
        'fn validate_agent_status(value: &str) -> Result<()> {\n',
        'fn agent_status_holds_task_lease(value: &str) -> bool {\n    matches!(value, "pending" | "running" | "idle")\n}\n\nfn validate_agent_status(value: &str) -> Result<()> {\n',
        "runtime lease status helper",
    )
    RUNTIME.write_text(text)


def patch_lease() -> None:
    text = LEASE.read_text()
    text = replace_once(
        text,
        'use std::fs;\nuse std::path::Path;\n',
        'use std::fs;\nuse std::fs::File;\nuse std::fs::OpenOptions;\nuse std::fs::TryLockError;\nuse std::path::Path;\n',
        "lease file locking imports",
    )
    # OpenOptions is already imported on some baselines; fail rather than duplicate silently.
    text = text.replace('use std::fs::OpenOptions;\nuse std::fs::OpenOptions;\n', 'use std::fs::OpenOptions;\n')
    text = replace_once(
        text,
        'const METADATA_RETRY_DELAY: Duration = Duration::from_millis(8);\n',
        'const METADATA_RETRY_DELAY: Duration = Duration::from_millis(8);\nconst LEASE_LOCK_TIMEOUT: Duration = Duration::from_secs(8);\nconst LEASE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(25);\npub(crate) const DEFAULT_TASK_LEASE_TTL_SECONDS: u64 = 900;\n',
        "lease constants",
    )
    text = text.replace('#[arg(long, default_value_t = 900)]\n        ttl_seconds: u64,', '#[arg(long, default_value_t = DEFAULT_TASK_LEASE_TTL_SECONDS)]\n        ttl_seconds: u64,')
    if text.count('DEFAULT_TASK_LEASE_TTL_SECONDS)]') != 2:
        raise SystemExit("lease ttl defaults: expected two replacements")

    scope_block = '''pub fn task_scope(task_id: &str) -> Result<String> {
    validate_token(task_id, "task id")?;
    Ok(format!("task-{task_id}"))
}
'''
    helpers = scope_block + '''
pub(crate) fn acquire_task(
    project_root: &Path,
    task_id: &str,
    owner: &str,
    ttl_seconds: u64,
) -> Result<()> {
    let scope = task_scope(task_id)?;
    acquire(project_root, &scope, owner, Some(task_id), ttl_seconds)?;
    Ok(())
}

pub(crate) fn renew_task(
    project_root: &Path,
    task_id: &str,
    owner: &str,
    ttl_seconds: u64,
) -> Result<()> {
    let scope = task_scope(task_id)?;
    renew(project_root, &scope, owner, ttl_seconds)?;
    Ok(())
}

pub(crate) fn release_task_if_owned(
    project_root: &Path,
    task_id: &str,
    owner: &str,
) -> Result<bool> {
    let scope = task_scope(task_id)?;
    validate_token(owner, "lease owner")?;
    let base = lease_base_dir(project_root)?;
    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    let _guard = ScopeLock::acquire(&base, &scope)?;
    let directory = scope_dir(&base, &scope);
    if !directory.is_dir() {
        return Ok(false);
    }
    let lease = load_record_retry(&directory)?;
    if expired(&lease) {
        remove_scope_dir(&directory, "expired lease")?;
        return Ok(false);
    }
    if lease.owner != owner {
        bail!("lease {scope} is owned by {}, not {owner}", lease.owner);
    }
    remove_scope_dir(&directory, "lease")?;
    Ok(true)
}
'''
    text = replace_once(text, scope_block, helpers, "lease task helpers")

    text = replace_once(
        text,
        '    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;\n    let scope_dir = scope_dir(&base, scope);\n\n    for _ in 0..4 {\n',
        '    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;\n    let _guard = ScopeLock::acquire(&base, scope)?;\n    let scope_dir = scope_dir(&base, scope);\n\n    for _ in 0..4 {\n',
        "lease acquire scope lock",
    )
    text = replace_once(
        text,
        '                    return renew(project_root, scope, owner, ttl_seconds);\n',
        '                    return renew_locked(&scope_dir, scope, owner, ttl_seconds);\n',
        "lease acquire same-owner renew",
    )

    renew_new = '''fn renew(
    project_root: &Path,
    scope: &str,
    owner: &str,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    validate_token(scope, "lease scope")?;
    validate_token(owner, "lease owner")?;
    let ttl_seconds = validate_ttl(ttl_seconds)?;
    let base = lease_base_dir(project_root)?;
    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    let _guard = ScopeLock::acquire(&base, scope)?;
    let directory = scope_dir(&base, scope);
    renew_locked(&directory, scope, owner, ttl_seconds)
}

fn renew_locked(
    directory: &Path,
    scope: &str,
    owner: &str,
    ttl_seconds: u64,
) -> Result<LeaseRecord> {
    let mut lease = load_record_retry(directory)?;
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
    atomic_write_record(&record_path(directory), &lease)?;
    Ok(lease)
}

'''
    text = replace_between(text, 'fn renew(\n', 'fn release(', renew_new, "lease renew")

    release_new = '''fn release(project_root: &Path, scope: &str, owner: &str) -> Result<()> {
    validate_token(scope, "lease scope")?;
    validate_token(owner, "lease owner")?;
    let base = lease_base_dir(project_root)?;
    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    let _guard = ScopeLock::acquire(&base, scope)?;
    let directory = scope_dir(&base, scope);
    let lease = load_record_retry(&directory)?;
    if lease.owner != owner {
        bail!("lease {scope} is owned by {}, not {owner}", lease.owner);
    }
    remove_scope_dir(&directory, "lease")
}

'''
    text = replace_between(text, 'fn release(', 'fn list(', release_new, "lease release")

    prune_new = '''fn prune(project_root: &Path) -> Result<Vec<String>> {
    let base = lease_base_dir(project_root)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut scopes = Vec::new();
    for entry in fs::read_dir(&base).with_context(|| format!("read {}", base.display()))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.path().is_dir() || entry.file_name() == ".locks" {
            continue;
        }
        if let Some(scope) = entry.file_name().to_str() {
            scopes.push(scope.to_string());
        }
    }
    let mut removed = Vec::new();
    for scope in scopes {
        if validate_token(&scope, "lease scope").is_err() {
            continue;
        }
        let _guard = ScopeLock::acquire(&base, &scope)?;
        let directory = scope_dir(&base, &scope);
        if !directory.is_dir() {
            continue;
        }
        let lease = match load_record_retry(&directory) {
            Ok(lease) => lease,
            Err(_) => continue,
        };
        if expired(&lease) {
            remove_scope_dir(&directory, "expired lease")?;
            removed.push(lease.scope);
        }
    }
    removed.sort();
    Ok(removed)
}

'''
    text = replace_between(text, 'fn prune(', 'fn lease_base_dir(', prune_new, "lease prune")

    lock_support = '''fn remove_scope_dir(directory: &Path, label: &str) -> Result<()> {
    match fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {label} {}", directory.display())),
    }
}

struct ScopeLock {
    _file: File,
}

impl ScopeLock {
    fn acquire(base: &Path, scope: &str) -> Result<Self> {
        validate_token(scope, "lease scope")?;
        let lock_dir = base.join(".locks");
        fs::create_dir_all(&lock_dir).with_context(|| format!("create {}", lock_dir.display()))?;
        let path = lock_dir.join(format!("{scope}.lock"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("open lease lock {}", path.display()))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => {
                    if started.elapsed() >= LEASE_LOCK_TIMEOUT {
                        bail!("timed out waiting for lease lock {}", path.display());
                    }
                    thread::sleep(LEASE_LOCK_RETRY_DELAY);
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("lock lease scope {}", path.display()));
                }
            }
        }
    }
}

'''
    text = replace_once(text, 'fn git_output(project_root: &Path, args: &[&str]) -> Result<String> {\n', lock_support + 'fn git_output(project_root: &Path, args: &[&str]) -> Result<String> {\n', "lease scope lock support")

    # Existing list() naturally ignores .locks because it has no lease.json, but
    # make the intent explicit and avoid four retry sleeps per list invocation.
    text = replace_once(
        text,
        '        if !entry.path().is_dir() {\n            continue;\n        }\n        let lease = match load_record_retry(&entry.path()) {\n',
        '        if !entry.path().is_dir() || entry.file_name() == ".locks" {\n            continue;\n        }\n        let lease = match load_record_retry(&entry.path()) {\n',
        "lease list lock directory skip",
    )
    LEASE.write_text(text)


def main() -> None:
    patch_cli()
    patch_runtime()
    patch_lease()
    print("ownership/resume transformations applied")


if __name__ == "__main__":
    main()
