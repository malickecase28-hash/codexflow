from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one patch target, found {count}: {old[:100]!r}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def replace_region(path: Path, start_marker: str, end_marker: str, replacement: str) -> None:
    text = path.read_text(encoding="utf-8")
    start = text.find(start_marker)
    if start < 0:
        raise SystemExit(f"{path}: missing start marker {start_marker!r}")
    end = text.find(end_marker, start)
    if end < 0:
        raise SystemExit(f"{path}: missing end marker {end_marker!r}")
    path.write_text(text[:start] + replacement + text[end:], encoding="utf-8")


# Public CodexFlow event command. Keep supervisor logic in one binary and delegate to it.
event_path = Path("codex-rs/cli/src/bin/codexflow/event.rs")
event_path.write_text(
    r'''use super::sibling_executable;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Args)]
pub(super) struct EventArgs {
    #[arg(long)]
    pub(super) project: Option<String>,
    #[command(subcommand)]
    command: EventCommand,
}

#[derive(Debug, Subcommand)]
enum EventCommand {
    Run {
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: String,
    },
    Publish {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        dedupe_key: Option<String>,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    Await {
        #[arg(long)]
        id: String,
        #[arg(long)]
        owner: String,
        #[arg(long = "topic", required = true)]
        topics: Vec<String>,
        #[arg(long)]
        key: Option<String>,
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
        #[arg(long)]
        timeout_at: Option<String>,
    },
    Inbox {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        clear: bool,
    },
    Status,
}

pub(super) fn handle(project_root: &Path, args: EventArgs) -> Result<()> {
    let executable = sibling_executable("codexflow-supervisor")?;
    let status = Command::new(&executable)
        .args(command_args(project_root, &args.command))
        .status()
        .with_context(|| format!("run {}", executable.display()))?;
    if !status.success() {
        bail!("codexflow-supervisor exited with {status}");
    }
    Ok(())
}

fn command_args(project_root: &Path, command: &EventCommand) -> Vec<OsString> {
    let mut args = Vec::new();
    match command {
        EventCommand::Run { bind } => {
            args.extend([OsString::from("run"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--bind"), OsString::from(bind)]);
        }
        EventCommand::Publish { kind, key, dedupe_key, payload } => {
            args.extend([OsString::from("publish"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--kind"), OsString::from(kind)]);
            if let Some(key) = key {
                args.extend([OsString::from("--key"), OsString::from(key)]);
            }
            if let Some(dedupe_key) = dedupe_key {
                args.extend([OsString::from("--dedupe-key"), OsString::from(dedupe_key)]);
            }
            args.extend([OsString::from("--payload"), OsString::from(payload)]);
        }
        EventCommand::Await { id, owner, topics, key, after_seq, timeout_at } => {
            args.extend([OsString::from("await"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--id"), OsString::from(id)]);
            args.extend([OsString::from("--owner"), OsString::from(owner)]);
            for topic in topics {
                args.extend([OsString::from("--topic"), OsString::from(topic)]);
            }
            if let Some(key) = key {
                args.extend([OsString::from("--key"), OsString::from(key)]);
            }
            args.extend([OsString::from("--after-seq"), OsString::from(after_seq.to_string())]);
            if let Some(timeout_at) = timeout_at {
                args.extend([OsString::from("--timeout-at"), OsString::from(timeout_at)]);
            }
        }
        EventCommand::Inbox { owner, clear } => {
            args.extend([OsString::from("inbox"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--owner"), OsString::from(owner)]);
            if *clear {
                args.push(OsString::from("--clear"));
            }
        }
        EventCommand::Status => {
            args.extend([OsString::from("status"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_args_forward_dedupe_key() {
        let rendered = command_args(
            Path::new("/repo"),
            &EventCommand::Publish {
                kind: "build.completed".to_string(),
                key: Some("job-1".to_string()),
                dedupe_key: Some("build:job-1".to_string()),
                payload: "{}".to_string(),
            },
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        assert!(rendered.windows(2).any(|pair| pair[0] == "--dedupe-key" && pair[1] == "build:job-1"));
    }

    #[test]
    fn await_args_forward_timeout_deadline() {
        let rendered = command_args(
            Path::new("/repo"),
            &EventCommand::Await {
                id: "wait-1".to_string(),
                owner: "god".to_string(),
                topics: vec!["build.completed".to_string()],
                key: None,
                after_seq: 7,
                timeout_at: Some("2030-01-01T00:00:00Z".to_string()),
            },
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        assert!(rendered.windows(2).any(|pair| pair[0] == "--timeout-at" && pair[1] == "2030-01-01T00:00:00Z"));
    }
}
''',
    encoding="utf-8",
)

cli = Path("codex-rs/cli/src/bin/codexflow.rs")
replace_once(cli, '#[path = "codexflow/delivery.rs"]\nmod delivery;\n', '#[path = "codexflow/delivery.rs"]\nmod delivery;\n#[path = "codexflow/event.rs"]\nmod event_runtime;\n')
replace_once(cli, '    Delivery(delivery::DeliveryArgs),\n    Caretaker(caretaker::CaretakerArgs),\n', '    Delivery(delivery::DeliveryArgs),\n    Event(event_runtime::EventArgs),\n    Caretaker(caretaker::CaretakerArgs),\n')
replace_once(
    cli,
    '                Some(TopCommand::Delivery(args)) => {\n                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;\n                    delivery::handle(&primary_root(&project)?, args)\n                }\n                Some(TopCommand::Caretaker(args)) => {',
    '                Some(TopCommand::Delivery(args)) => {\n                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;\n                    delivery::handle(&primary_root(&project)?, args)\n                }\n                Some(TopCommand::Event(args)) => {\n                    let project = resolve_scoped_project(&runtime, args.project.as_deref()).await?;\n                    event_runtime::handle(&primary_root(&project)?, args)\n                }\n                Some(TopCommand::Caretaker(args)) => {',
)
replace_once(
    cli,
    '''fn sibling_codex_executable() -> Result<PathBuf> {
    let current = std::env::current_exe().context("resolve codexflow executable")?;
    let sibling_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(parent) = current.parent() {
        let sibling = parent.join(sibling_name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    let path = which::which("codex").context("find codex executable on PATH")?;
    if path == current {
        bail!("resolved codex executable points back to codexflow");
    }
    Ok(path)
}
''',
    '''fn sibling_executable(name: &str) -> Result<PathBuf> {
    let current = std::env::current_exe().context("resolve codexflow executable")?;
    let sibling_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(parent) = current.parent() {
        let sibling = parent.join(&sibling_name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    let path = which::which(name).with_context(|| format!("find {name} executable on PATH"))?;
    if path == current {
        bail!("resolved {name} executable points back to codexflow");
    }
    Ok(path)
}

fn sibling_codex_executable() -> Result<PathBuf> {
    sibling_executable("codex")
}
''',
)
replace_once(
    cli,
    '''    #[test]
    fn idempotency_key_uses_primary_root() {
''',
    '''    #[test]
    fn event_command_is_exposed_on_public_cli() {
        let cli = Cli::try_parse_from(["codexflow", "event", "--project", "demo", "status"])
            .expect("event command should parse");
        let Some(TopCommand::Event(args)) = cli.command else {
            panic!("expected event command");
        };
        assert_eq!(args.project.as_deref(), Some("demo"));
    }

    #[test]
    fn idempotency_key_uses_primary_root() {
''',
)

supervisor = Path("codex-rs/cli/src/bin/codexflow-supervisor.rs")
replace_once(supervisor, 'use chrono::Utc;\n', 'use chrono::DateTime;\nuse chrono::Utc;\n')
replace_once(supervisor, 'use std::net::TcpListener;\nuse std::net::TcpStream;\n', 'use std::net::SocketAddr;\nuse std::net::TcpListener;\nuse std::net::TcpStream;\n')
replace_once(supervisor, 'use std::path::PathBuf;\n', 'use std::path::PathBuf;\n#[cfg(windows)]\nuse std::os::windows::ffi::OsStrExt;\n')
replace_once(supervisor, 'const SUPERVISOR_IO_TIMEOUT: Duration = Duration::from_secs(30);\n', 'const SUPERVISOR_IO_TIMEOUT: Duration = Duration::from_secs(30);\nconst TIMER_TICK: Duration = Duration::from_millis(50);\n')
replace_once(supervisor, '        #[arg(long)]\n        key: Option<String>,\n        #[arg(long, default_value = "{}")]', '        #[arg(long)]\n        key: Option<String>,\n        #[arg(long)]\n        dedupe_key: Option<String>,\n        #[arg(long, default_value = "{}")]',)
replace_once(supervisor, '        #[arg(long, default_value_t = 0)]\n        after_seq: u64,\n    },\n    Inbox {', '        #[arg(long, default_value_t = 0)]\n        after_seq: u64,\n        #[arg(long)]\n        timeout_at: Option<String>,\n    },\n    Inbox {')
replace_once(supervisor, '    key: Option<String>,\n    payload: Value,\n    created_at: String,', '    key: Option<String>,\n    #[serde(default)]\n    dedupe_key: Option<String>,\n    payload: Value,\n    created_at: String,')
replace_once(supervisor, '    after_seq: u64,\n    state: String,', '    after_seq: u64,\n    #[serde(default)]\n    timeout_at: Option<String>,\n    state: String,')
replace_once(supervisor, '        key: Option<String>,\n        payload: Value,\n    },\n    RegisterAwait {', '        key: Option<String>,\n        #[serde(default)]\n        dedupe_key: Option<String>,\n        payload: Value,\n    },\n    RegisterAwait {')
replace_once(supervisor, '        key: Option<String>,\n        after_seq: u64,\n    },\n    Inbox {', '        key: Option<String>,\n        after_seq: u64,\n        #[serde(default)]\n        timeout_at: Option<String>,\n    },\n    Inbox {')
replace_once(supervisor, 'enum SupervisorSendError {\n    NotDispatched(anyhow::Error),\n    DoNotReplay(anyhow::Error),\n}', 'enum SupervisorSendError {\n    NotDispatched,\n    DoNotReplay(anyhow::Error),\n}')
replace_once(supervisor, '            key,\n            payload,\n        } => {', '            key,\n            dedupe_key,\n            payload,\n        } => {')
replace_once(supervisor, '                WireRequest::Publish { kind, key, payload },', '                WireRequest::Publish {\n                    kind,\n                    key,\n                    dedupe_key,\n                    payload,\n                },')
replace_once(supervisor, '            key,\n            after_seq,\n        } => {', '            key,\n            after_seq,\n            timeout_at,\n        } => {')
replace_once(
    supervisor,
    '''                WireRequest::RegisterAwait {
                    id,
                    owner,
                    topics,
                    key,
                    after_seq,
                },
''',
    '''                WireRequest::RegisterAwait {
                    id,
                    owner,
                    topics,
                    key,
                    after_seq,
                    timeout_at,
                },
''',
)

replace_region(
    supervisor,
    'fn run_supervisor(project_root: &Path, bind: &str) -> Result<()> {',
    '\nfn handle_connection(',
    '''fn run_supervisor(project_root: &Path, bind: &str) -> Result<()> {
    ensure_state_dirs(project_root)?;
    reconcile_waiting_awaits(project_root)?;
    process_due_timeouts(project_root)?;
    let bind_address = parse_loopback_address(bind)?;
    let listener = TcpListener::bind(bind_address)
        .with_context(|| format!("bind supervisor to {bind_address}"))?;
    listener
        .set_nonblocking(true)
        .context("set supervisor listener nonblocking")?;
    let address = listener.local_addr().context("read supervisor address")?;
    let endpoint = SupervisorEndpoint {
        schema: ENDPOINT_SCHEMA.to_string(),
        address: address.to_string(),
        pid: std::process::id(),
        started_at: now_iso(),
    };
    write_json_atomic(&endpoint_path(project_root), &endpoint)?;
    println!("{}", serde_json::to_string_pretty(&endpoint)?);

    loop {
        process_due_timeouts(project_root)?;
        match listener.accept() {
            Ok((stream, _)) => {
                let root = project_root.to_path_buf();
                thread::spawn(move || {
                    if let Err(err) = handle_connection(&root, stream) {
                        eprintln!("CodexFlow supervisor request failed: {err:#}");
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(TIMER_TICK),
            Err(err) => {
                eprintln!("CodexFlow supervisor accept failed: {err}");
                thread::sleep(TIMER_TICK);
            }
        }
    }
}
''',
)
replace_once(supervisor, 'fn handle_connection(project_root: &Path, mut stream: TcpStream) -> Result<()> {\n    let read_stream = stream.try_clone().context("clone supervisor socket")?;', 'fn handle_connection(project_root: &Path, mut stream: TcpStream) -> Result<()> {\n    stream.set_read_timeout(Some(SUPERVISOR_IO_TIMEOUT)).context("set supervisor request read timeout")?;\n    stream.set_write_timeout(Some(SUPERVISOR_IO_TIMEOUT)).context("set supervisor response write timeout")?;\n    let read_stream = stream.try_clone().context("clone supervisor socket")?;')
replace_once(supervisor, 'Err(SupervisorSendError::NotDispatched(_)) => {}', 'Err(SupervisorSendError::NotDispatched) => {}')

replace_region(
    supervisor,
    'fn send_request(\n',
    '\nfn execute_request(',
    '''fn send_request(
    endpoint: &SupervisorEndpoint,
    request: &WireRequest,
) -> std::result::Result<Value, SupervisorSendError> {
    if endpoint.schema != ENDPOINT_SCHEMA {
        return Err(SupervisorSendError::NotDispatched);
    }
    let address = parse_loopback_address(&endpoint.address)
        .map_err(|_| SupervisorSendError::NotDispatched)?;
    let encoded = serde_json::to_string(request).map_err(|_| SupervisorSendError::NotDispatched)?;
    let mut stream = TcpStream::connect(address).map_err(|_| SupervisorSendError::NotDispatched)?;
    stream
        .set_read_timeout(Some(SUPERVISOR_IO_TIMEOUT))
        .map_err(|_| SupervisorSendError::NotDispatched)?;
    stream
        .set_write_timeout(Some(SUPERVISOR_IO_TIMEOUT))
        .map_err(|_| SupervisorSendError::NotDispatched)?;
    writeln!(stream, "{encoded}").map_err(|err| SupervisorSendError::DoNotReplay(err.into()))?;
    stream.flush().map_err(|err| SupervisorSendError::DoNotReplay(err.into()))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .map_err(|err| SupervisorSendError::DoNotReplay(anyhow::Error::new(err)))?;
    if bytes == 0 || line.trim().is_empty() {
        return Err(SupervisorSendError::DoNotReplay(anyhow::Error::msg(
            "supervisor closed the connection without a response",
        )));
    }
    let response: WireResponse = serde_json::from_str(line.trim_end()).map_err(|err| {
        SupervisorSendError::DoNotReplay(anyhow::Error::new(err).context("parse supervisor response"))
    })?;
    if !response.ok {
        return Err(SupervisorSendError::DoNotReplay(anyhow::Error::msg(
            response.error.unwrap_or_else(|| "supervisor request failed".to_string()),
        )));
    }
    Ok(response.value.unwrap_or(Value::Null))
}
''',
)

replace_region(
    supervisor,
    'fn execute_request(project_root: &Path, request: WireRequest) -> Result<Value> {',
    '\nfn publish_event(',
    '''fn execute_request(project_root: &Path, request: WireRequest) -> Result<Value> {
    ensure_state_dirs(project_root)?;
    match request {
        WireRequest::Publish { kind, key, dedupe_key, payload } => {
            validate_event_kind(&kind)?;
            validate_key(key.as_deref())?;
            validate_dedupe_key(dedupe_key.as_deref())?;
            publish_event_with_dedupe(project_root, kind, key, dedupe_key, payload)
        }
        WireRequest::RegisterAwait { id, owner, topics, key, after_seq, timeout_at } => {
            register_await_with_timeout(project_root, id, owner, topics, key, after_seq, timeout_at)
        }
        WireRequest::Inbox { owner, clear } => read_inbox(project_root, &owner, clear),
        WireRequest::Status => with_state_lock(project_root, || supervisor_status(project_root)),
    }
}
''',
)

replace_region(
    supervisor,
    'fn publish_event(\n',
    '\nfn register_await(',
    '''fn publish_event(
    project_root: &Path,
    kind: String,
    key: Option<String>,
    payload: Value,
) -> Result<Value> {
    publish_event_with_dedupe(project_root, kind, key, None, payload)
}

fn publish_event_with_dedupe(
    project_root: &Path,
    kind: String,
    key: Option<String>,
    dedupe_key: Option<String>,
    payload: Value,
) -> Result<Value> {
    with_state_lock(project_root, || {
        let (event, duplicate) = append_or_reuse_event_unlocked(
            project_root,
            kind,
            key,
            dedupe_key,
            payload,
        )?;
        let matched = resolve_event_against_awaits(project_root, &event)?;
        Ok(json!({ "event": event, "matched_awaits": matched, "duplicate": duplicate }))
    })
}

fn append_or_reuse_event_unlocked(
    project_root: &Path,
    kind: String,
    key: Option<String>,
    dedupe_key: Option<String>,
    payload: Value,
) -> Result<(EventEnvelope, bool)> {
    let events: Vec<EventEnvelope> = read_json_lines(&events_path(project_root))?;
    if let Some(dedupe_key) = dedupe_key.as_deref()
        && let Some(existing) = events
            .iter()
            .find(|event| event.dedupe_key.as_deref() == Some(dedupe_key))
    {
        if existing.kind != kind || existing.key != key || existing.payload != payload {
            bail!("event dedupe key already exists with different content: {dedupe_key}");
        }
        return Ok((existing.clone(), true));
    }
    let seq = events.last().map_or(0, |event| event.seq).saturating_add(1);
    let event = EventEnvelope {
        schema: EVENT_SCHEMA.to_string(),
        seq,
        id: format!("ev-{seq}"),
        kind,
        key,
        dedupe_key,
        payload,
        created_at: now_iso(),
    };
    append_json_line(&events_path(project_root), &event)?;
    Ok((event, false))
}
''',
)

replace_region(
    supervisor,
    'fn register_await(\n',
    '\nfn read_inbox(',
    '''fn register_await(
    project_root: &Path,
    id: String,
    owner: String,
    topics: Vec<String>,
    key: Option<String>,
    after_seq: u64,
) -> Result<Value> {
    register_await_with_timeout(project_root, id, owner, topics, key, after_seq, None)
}

fn register_await_with_timeout(
    project_root: &Path,
    id: String,
    owner: String,
    topics: Vec<String>,
    key: Option<String>,
    after_seq: u64,
    timeout_at: Option<String>,
) -> Result<Value> {
    validate_id(&id)?;
    validate_owner(&owner)?;
    validate_topics(&topics)?;
    validate_key(key.as_deref())?;
    let timeout_at = normalize_timeout_at(timeout_at.as_deref())?;
    with_state_lock(project_root, || {
        let mut awaits = load_awaits(project_root)?;
        if let Some(existing) = awaits.get(&id) {
            if same_await_registration_with_timeout(
                existing,
                &owner,
                &topics,
                key.as_deref(),
                after_seq,
                timeout_at.as_deref(),
            ) {
                return Ok(serde_json::to_value(existing)?);
            }
            bail!("await id already exists with different configuration: {id}");
        }
        let now = now_iso();
        awaits.insert(
            id.clone(),
            AwaitSpec {
                schema: AWAIT_SCHEMA.to_string(),
                id: id.clone(),
                owner,
                topics,
                key,
                after_seq,
                timeout_at,
                state: "waiting".to_string(),
                matched_event_id: None,
                created_at: now.clone(),
                updated_at: now,
            },
        );
        save_awaits(project_root, &awaits)?;
        resolve_existing_events_for_await(project_root, &id, &mut awaits)?;
        save_awaits(project_root, &awaits)?;
        Ok(serde_json::to_value(
            awaits.get(&id).context("await disappeared after registration")?,
        )?)
    })
}

fn same_await_registration(
    existing: &AwaitSpec,
    owner: &str,
    topics: &[String],
    key: Option<&str>,
    after_seq: u64,
) -> bool {
    same_await_registration_with_timeout(existing, owner, topics, key, after_seq, None)
}

fn same_await_registration_with_timeout(
    existing: &AwaitSpec,
    owner: &str,
    topics: &[String],
    key: Option<&str>,
    after_seq: u64,
    timeout_at: Option<&str>,
) -> bool {
    existing.owner == owner
        && existing.topics == topics
        && existing.key.as_deref() == key
        && existing.after_seq == after_seq
        && existing.timeout_at.as_deref() == timeout_at
}
''',
)

replace_once(supervisor, '        if clear && path.exists() {\n            fs::write(&path, b"").with_context(|| format!("clear {}", path.display()))?;\n        }', '        if clear && path.exists() {\n            let file = OpenOptions::new().write(true).truncate(true).open(&path)\n                .with_context(|| format!("clear {}", path.display()))?;\n            file.sync_all().with_context(|| format!("sync {}", path.display()))?;\n        }')

insert_marker = 'fn reconcile_waiting_awaits(project_root: &Path) -> Result<()> {'
text = supervisor.read_text(encoding="utf-8")
idx = text.find(insert_marker)
if idx < 0:
    raise SystemExit("supervisor: missing reconciliation marker")
timer_code = r'''fn process_due_timeouts(project_root: &Path) -> Result<usize> {
    with_state_lock(project_root, || {
        let mut awaits = load_awaits(project_root)?;
        let now = Utc::now();
        let mut due_ids = Vec::new();
        for (id, spec) in &awaits {
            if spec.state != "waiting" {
                continue;
            }
            if let Some(timeout_at) = spec.timeout_at.as_deref() {
                let deadline = DateTime::parse_from_rfc3339(timeout_at)
                    .with_context(|| format!("invalid persisted timeout deadline for await {id}: {timeout_at}"))?
                    .with_timezone(&Utc);
                if deadline <= now {
                    due_ids.push(id.clone());
                }
            }
        }
        for id in &due_ids {
            let owner = awaits.get(id).context("due await disappeared")?.owner.clone();
            let (event, _) = append_or_reuse_event_unlocked(
                project_root,
                "timer.elapsed".to_string(),
                Some(id.clone()),
                Some(format!("await-timeout:{id}")),
                json!({ "await_id": id, "owner": owner }),
            )?;
            if awaits.get(id).is_some_and(|spec| spec.state == "waiting") {
                fire_await(project_root, &mut awaits, id, &event)?;
            }
        }
        save_awaits(project_root, &awaits)?;
        Ok(due_ids.len())
    })
}

'''
supervisor.write_text(text[:idx] + timer_code + text[idx:], encoding="utf-8")

replace_region(
    supervisor,
    'fn resolve_event_against_awaits(project_root: &Path, event: &EventEnvelope) -> Result<Vec<String>> {',
    '\nfn fire_await(',
    '''fn resolve_event_against_awaits(project_root: &Path, event: &EventEnvelope) -> Result<Vec<String>> {
    let mut awaits = load_awaits(project_root)?;
    let matching_ids = awaits
        .iter()
        .filter(|(_, spec)| {
            spec.state == "waiting"
                && event.seq > spec.after_seq
                && spec.topics.iter().any(|topic| topic_matches(topic, &event.kind))
                && event_key_matches(spec.key.as_deref(), event.key.as_deref())
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &matching_ids {
        fire_await(project_root, &mut awaits, id, event)?;
    }
    save_awaits(project_root, &awaits)?;
    Ok(matching_ids)
}
''',
)

text = supervisor.read_text(encoding="utf-8")
marker = 'fn canonical_project_root(path: PathBuf) -> Result<PathBuf> {'
idx = text.find(marker)
if idx < 0:
    raise SystemExit("supervisor: missing canonical root marker")
validation = r'''fn validate_dedupe_key(dedupe_key: Option<&str>) -> Result<()> {
    if let Some(dedupe_key) = dedupe_key
        && (dedupe_key.is_empty()
            || dedupe_key.len() > 256
            || dedupe_key.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0')))
    {
        bail!("invalid event dedupe key");
    }
    Ok(())
}

fn normalize_timeout_at(timeout_at: Option<&str>) -> Result<Option<String>> {
    timeout_at
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .with_context(|| format!("invalid RFC3339 timeout deadline: {value}"))
                .map(|deadline| deadline.with_timezone(&Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        })
        .transpose()
}

fn parse_loopback_address(address: &str) -> Result<SocketAddr> {
    let address: SocketAddr = address
        .parse()
        .with_context(|| format!("parse supervisor address {address:?}"))?;
    if !address.ip().is_loopback() {
        bail!("supervisor address must be loopback-only: {address}");
    }
    Ok(address)
}

'''
supervisor.write_text(text[:idx] + validation + text[idx:], encoding="utf-8")
replace_once(supervisor, '    if endpoint.schema != ENDPOINT_SCHEMA {\n        bail!("unsupported supervisor endpoint schema {}", endpoint.schema);\n    }\n    Ok(endpoint)', '    if endpoint.schema != ENDPOINT_SCHEMA {\n        bail!("unsupported supervisor endpoint schema {}", endpoint.schema);\n    }\n    parse_loopback_address(&endpoint.address)?;\n    Ok(endpoint)')

replace_region(
    supervisor,
    'fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {',
    '\nfn with_state_lock',
    r'''fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(value)?)?;
    file.flush()?;
    file.sync_data().with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .with_context(|| format!("open {}", tmp.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    file.sync_all().with_context(|| format!("sync {}", tmp.display()))?;
    drop(file);
    replace_file(&tmp, path)?;
    sync_parent_directory(path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING;
    use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let destination_display = destination.display().to_string();
    let source = source.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let destination = destination.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("replace {destination_display}"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).with_context(|| format!("replace {}", destination.display()))
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .with_context(|| format!("open {}", parent.display()))?
            .sync_all()
            .with_context(|| format!("sync {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<()> {
    Ok(())
}
''',
)

replace_once(supervisor, '            key: key.map(str::to_string),\n            payload: Value::Null,', '            key: key.map(str::to_string),\n            dedupe_key: None,\n            payload: Value::Null,')
replace_once(supervisor, '            after_seq: 7,\n            state: "waiting".to_string(),', '            after_seq: 7,\n            timeout_at: None,\n            state: "waiting".to_string(),')
replace_once(supervisor, '                key: Some("worker-1".to_string()),\n                payload: Value::Null,', '                key: Some("worker-1".to_string()),\n                dedupe_key: None,\n                payload: Value::Null,')

text = supervisor.read_text(encoding="utf-8")
marker = '    #[test]\n    fn wildcard_topic_matches_expected_event_kinds() {'
idx = text.find(marker)
if idx < 0:
    raise SystemExit("supervisor: missing test insertion marker")
new_tests = r'''    #[test]
    fn loopback_address_validation_blocks_remote_binding() {
        assert!(parse_loopback_address("127.0.0.1:0").is_ok());
        assert!(parse_loopback_address("[::1]:0").is_ok());
        assert!(parse_loopback_address("0.0.0.0:7777").is_err());
        assert!(parse_loopback_address("192.0.2.1:7777").is_err());
    }

    #[test]
    fn duplicate_publish_reuses_event_and_delivers_once() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        register_await(
            root,
            "build_wait".to_string(),
            "god".to_string(),
            vec!["build.completed".to_string()],
            Some("job-1".to_string()),
            0,
        )
        .expect("register await");
        let first = publish_event_with_dedupe(
            root,
            "build.completed".to_string(),
            Some("job-1".to_string()),
            Some("build:job-1".to_string()),
            json!({"status":"ok"}),
        )
        .expect("first publish");
        let second = publish_event_with_dedupe(
            root,
            "build.completed".to_string(),
            Some("job-1".to_string()),
            Some("build:job-1".to_string()),
            json!({"status":"ok"}),
        )
        .expect("duplicate publish");
        assert_eq!(first["duplicate"], Value::Bool(false));
        assert_eq!(second["duplicate"], Value::Bool(true));
        let events: Vec<EventEnvelope> = read_json_lines(&events_path(root)).expect("read events");
        assert_eq!(events.len(), 1);
        let inbox: Vec<InboxRecord> = read_json_lines(&inbox_path(root, "god")).expect("read inbox");
        assert_eq!(inbox.len(), 1);
    }

    #[test]
    fn dedupe_key_cannot_alias_different_event_content() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        publish_event_with_dedupe(
            root,
            "build.completed".to_string(),
            Some("job-1".to_string()),
            Some("build:job-1".to_string()),
            json!({"status":"ok"}),
        )
        .expect("first publish");
        let err = publish_event_with_dedupe(
            root,
            "build.completed".to_string(),
            Some("job-1".to_string()),
            Some("build:job-1".to_string()),
            json!({"status":"failed"}),
        )
        .expect_err("conflicting dedupe key must fail");
        assert!(err.to_string().contains("different content"));
    }

    #[test]
    fn due_timeout_is_durable_and_fires_once() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        let deadline = (Utc::now() - chrono::Duration::seconds(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        register_await_with_timeout(
            root,
            "timeout_wait".to_string(),
            "god".to_string(),
            vec!["build.completed".to_string()],
            None,
            0,
            Some(deadline),
        )
        .expect("register timed await");
        assert_eq!(process_due_timeouts(root).expect("process timeout"), 1);
        assert_eq!(process_due_timeouts(root).expect("repeat timeout pass"), 0);
        let events: Vec<EventEnvelope> = read_json_lines(&events_path(root)).expect("read timeout event");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "timer.elapsed");
        assert_eq!(events[0].dedupe_key.as_deref(), Some("await-timeout:timeout_wait"));
        let inbox: Vec<InboxRecord> = read_json_lines(&inbox_path(root, "god")).expect("read timeout inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].await_id, "timeout_wait");
    }

'''
supervisor.write_text(text[:idx] + new_tests + text[idx:], encoding="utf-8")

cargo = Path("codex-rs/cli/Cargo.toml")
replace_once(cargo, '    "Win32_Storage_Packaging_Appx",\n', '    "Win32_Storage_FileSystem",\n    "Win32_Storage_Packaging_Appx",\n')

windows = Path(".github/workflows/codexflow-prebuilt-windows.yml")
replace_once(windows, '      - uses: dtolnay/rust-toolchain@e081816240890017053eacbb1bdf337761dc5582\n        with:\n          toolchain: "1.95.0"', '      - uses: dtolnay/rust-toolchain@ce678459e9fc7500d337468f904b95f1b5c10b5e\n        with:\n          toolchain: "1.98.1"')
replace_once(
    windows,
    '''      - name: Build CodexFlow runtime bundle
        shell: pwsh
        run: |
''',
    '''      - name: Test CodexFlow runtime bundle
        shell: pwsh
        run: |
          $env:LIBSQLITE3_FLAGS = "SQLITE_DISABLE_INTRINSIC"
          cargo test --locked --target x86_64-pc-windows-msvc -p codex-cli --bin codexflow --bin codexflow-supervisor

      - name: Build CodexFlow runtime bundle
        shell: pwsh
        run: |
''',
)
