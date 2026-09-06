use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::DateTime;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const ENDPOINT_SCHEMA: &str = "codexflow.supervisor.endpoint.v1";
const EVENT_SCHEMA: &str = "codexflow.event.v1";
const AWAIT_SCHEMA: &str = "codexflow.await.v1";
const INBOX_SCHEMA: &str = "codexflow.inbox.v1";
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(8);
const SUPERVISOR_IO_TIMEOUT: Duration = Duration::from_secs(30);
const TIMER_TICK: Duration = Duration::from_millis(50);
const MAX_WIRE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "codexflow-supervisor",
    version,
    about = "Event-driven background supervisor for CodexFlow"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: String,
    },
    Publish {
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
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
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
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
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        clear: bool,
    },
    Status {
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SupervisorEndpoint {
    schema: String,
    address: String,
    pid: u32,
    started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventEnvelope {
    schema: String,
    seq: u64,
    id: String,
    kind: String,
    key: Option<String>,
    #[serde(default)]
    dedupe_key: Option<String>,
    payload: Value,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AwaitSpec {
    schema: String,
    id: String,
    owner: String,
    topics: Vec<String>,
    #[serde(default)]
    key: Option<String>,
    after_seq: u64,
    #[serde(default)]
    timeout_at: Option<String>,
    state: String,
    matched_event_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InboxRecord {
    schema: String,
    await_id: String,
    event: EventEnvelope,
    delivered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum WireRequest {
    Publish {
        kind: String,
        key: Option<String>,
        #[serde(default)]
        dedupe_key: Option<String>,
        payload: Value,
    },
    RegisterAwait {
        id: String,
        owner: String,
        topics: Vec<String>,
        key: Option<String>,
        after_seq: u64,
        #[serde(default)]
        timeout_at: Option<String>,
    },
    Inbox {
        owner: String,
        clear: bool,
    },
    Status,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireResponse {
    ok: bool,
    value: Option<Value>,
    error: Option<String>,
}

#[derive(Debug)]
enum SupervisorSendError {
    NotDispatched,
    DoNotReplay(anyhow::Error),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { project_root, bind } => {
            run_supervisor(&canonical_project_root(project_root)?, &bind)
        }
        Command::Publish {
            project_root,
            kind,
            key,
            dedupe_key,
            payload,
        } => {
            let root = canonical_project_root(project_root)?;
            validate_event_kind(&kind)?;
            let payload: Value =
                serde_json::from_str(&payload).context("parse --payload as JSON")?;
            print_value(execute_with_optional_supervisor(
                &root,
                WireRequest::Publish {
                    kind,
                    key,
                    dedupe_key,
                    payload,
                },
            )?)
        }
        Command::Await {
            project_root,
            id,
            owner,
            topics,
            key,
            after_seq,
            timeout_at,
        } => {
            let root = canonical_project_root(project_root)?;
            print_value(execute_with_optional_supervisor(
                &root,
                WireRequest::RegisterAwait {
                    id,
                    owner,
                    topics,
                    key,
                    after_seq,
                    timeout_at,
                },
            )?)
        }
        Command::Inbox {
            project_root,
            owner,
            clear,
        } => {
            let root = canonical_project_root(project_root)?;
            print_value(execute_with_optional_supervisor(
                &root,
                WireRequest::Inbox { owner, clear },
            )?)
        }
        Command::Status { project_root } => {
            let root = canonical_project_root(project_root)?;
            print_value(execute_with_optional_supervisor(
                &root,
                WireRequest::Status,
            )?)
        }
    }
}

fn run_supervisor(project_root: &Path, bind: &str) -> Result<()> {
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

fn handle_connection(project_root: &Path, mut stream: TcpStream) -> Result<()> {
    stream
        .set_nonblocking(false)
        .context("set accepted supervisor socket blocking")?;
    stream
        .set_read_timeout(Some(SUPERVISOR_IO_TIMEOUT))
        .context("set supervisor request read timeout")?;
    stream
        .set_write_timeout(Some(SUPERVISOR_IO_TIMEOUT))
        .context("set supervisor response write timeout")?;
    let read_stream = stream.try_clone().context("clone supervisor socket")?;
    let Some(request_line) = read_wire_request(BufReader::new(read_stream))? else {
        return Ok(());
    };

    let response = match serde_json::from_str::<WireRequest>(request_line.trim_end()) {
        Ok(request) => match execute_request(project_root, request) {
            Ok(value) => WireResponse {
                ok: true,
                value: Some(value),
                error: None,
            },
            Err(err) => WireResponse {
                ok: false,
                value: None,
                error: Some(format!("{err:#}")),
            },
        },
        Err(err) => WireResponse {
            ok: false,
            value: None,
            error: Some(format!("invalid request: {err}")),
        },
    };
    writeln!(stream, "{}", serde_json::to_string(&response)?)?;
    stream.flush()?;
    Ok(())
}

fn read_wire_request<R: BufRead>(reader: R) -> Result<Option<String>> {
    let mut limited = reader.take((MAX_WIRE_BYTES + 1) as u64);
    let mut request_line = String::new();
    let bytes = limited
        .read_line(&mut request_line)
        .context("read supervisor request")?;
    if bytes == 0 {
        return Ok(None);
    }
    if request_line.len() > MAX_WIRE_BYTES {
        bail!("supervisor request exceeds {MAX_WIRE_BYTES} bytes");
    }
    if !request_line.ends_with('\n') {
        bail!("supervisor request is not newline terminated");
    }
    Ok(Some(request_line))
}

fn execute_with_optional_supervisor(project_root: &Path, request: WireRequest) -> Result<Value> {
    if let Ok(endpoint) = read_endpoint(project_root) {
        match send_request(&endpoint, &request) {
            Ok(value) => return Ok(value),
            Err(SupervisorSendError::NotDispatched) => {}
            Err(SupervisorSendError::DoNotReplay(err)) => {
                return Err(err)
                    .context("supervisor request was dispatched; refusing unsafe local replay");
            }
        }
    }
    execute_request(project_root, request)
}

fn send_request(
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
    stream
        .flush()
        .map_err(|err| SupervisorSendError::DoNotReplay(err.into()))?;
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
        SupervisorSendError::DoNotReplay(
            anyhow::Error::new(err).context("parse supervisor response"),
        )
    })?;
    if !response.ok {
        return Err(SupervisorSendError::DoNotReplay(anyhow::Error::msg(
            response
                .error
                .unwrap_or_else(|| "supervisor request failed".to_string()),
        )));
    }
    Ok(response.value.unwrap_or(Value::Null))
}

fn execute_request(project_root: &Path, request: WireRequest) -> Result<Value> {
    ensure_state_dirs(project_root)?;
    match request {
        WireRequest::Publish {
            kind,
            key,
            dedupe_key,
            payload,
        } => {
            validate_event_kind(&kind)?;
            validate_key(key.as_deref())?;
            validate_dedupe_key(dedupe_key.as_deref())?;
            publish_event_with_dedupe(project_root, kind, key, dedupe_key, payload)
        }
        WireRequest::RegisterAwait {
            id,
            owner,
            topics,
            key,
            after_seq,
            timeout_at,
        } => {
            register_await_with_timeout(project_root, id, owner, topics, key, after_seq, timeout_at)
        }
        WireRequest::Inbox { owner, clear } => read_inbox(project_root, &owner, clear),
        WireRequest::Status => with_state_lock(project_root, || supervisor_status(project_root)),
    }
}

#[allow(dead_code)] // thin wrapper exercised by unit tests; dispatch uses the dedupe variant
fn publish_event(
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
        let (event, duplicate) =
            append_or_reuse_event_unlocked(project_root, kind, key, dedupe_key, payload)?;
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

#[allow(dead_code)] // thin wrapper exercised by unit tests; dispatch uses the timeout variant
fn register_await(
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
            awaits
                .get(&id)
                .context("await disappeared after registration")?,
        )?)
    })
}

#[allow(dead_code)] // thin wrapper exercised by unit tests; dispatch uses the timeout variant
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

fn read_inbox(project_root: &Path, owner: &str, clear: bool) -> Result<Value> {
    validate_owner(owner)?;
    with_state_lock(project_root, || {
        let path = inbox_path(project_root, owner);
        let records: Vec<InboxRecord> = read_json_lines(&path)?;
        if clear && path.exists() {
            let file = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .with_context(|| format!("clear {}", path.display()))?;
            file.sync_all()
                .with_context(|| format!("sync {}", path.display()))?;
        }
        Ok(json!({ "owner": owner, "records": records, "cleared": clear }))
    })
}

fn supervisor_status(project_root: &Path) -> Result<Value> {
    let endpoint = read_endpoint(project_root).ok();
    let awaits = load_awaits(project_root)?;
    let waiting = awaits
        .values()
        .filter(|spec| spec.state == "waiting")
        .count();
    let fired = awaits.values().filter(|spec| spec.state == "fired").count();
    Ok(json!({
        "project_root": project_root,
        "endpoint": endpoint,
        "last_event_seq": last_event_seq(project_root)?,
        "awaits": { "waiting": waiting, "fired": fired, "total": awaits.len() }
    }))
}

fn process_due_timeouts(project_root: &Path) -> Result<usize> {
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
                    .with_context(|| {
                        format!("invalid persisted timeout deadline for await {id}: {timeout_at}")
                    })?
                    .with_timezone(&Utc);
                if deadline <= now {
                    due_ids.push(id.clone());
                }
            }
        }
        for id in &due_ids {
            let owner = awaits
                .get(id)
                .context("due await disappeared")?
                .owner
                .clone();
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
        if !due_ids.is_empty() {
            save_awaits(project_root, &awaits)?;
        }
        Ok(due_ids.len())
    })
}

fn reconcile_waiting_awaits(project_root: &Path) -> Result<()> {
    with_state_lock(project_root, || {
        let mut awaits = load_awaits(project_root)?;
        let ids = awaits
            .iter()
            .filter(|(_, spec)| spec.state == "waiting")
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            resolve_existing_events_for_await(project_root, &id, &mut awaits)?;
        }
        save_awaits(project_root, &awaits)
    })
}

fn resolve_existing_events_for_await(
    project_root: &Path,
    await_id: &str,
    awaits: &mut BTreeMap<String, AwaitSpec>,
) -> Result<()> {
    let Some(spec) = awaits.get(await_id).cloned() else {
        return Ok(());
    };
    if spec.state != "waiting" {
        return Ok(());
    }
    let events: Vec<EventEnvelope> = read_json_lines(&events_path(project_root))?;
    if let Some(event) = events.into_iter().find(|event| {
        event.seq > spec.after_seq
            && spec
                .topics
                .iter()
                .any(|topic| topic_matches(topic, &event.kind))
            && event_key_matches(spec.key.as_deref(), event.key.as_deref())
            && spec
                .timeout_at
                .as_deref()
                .is_none_or(|deadline| event.created_at.as_str() <= deadline)
    }) {
        fire_await(project_root, awaits, await_id, &event)?;
    }
    Ok(())
}

fn resolve_event_against_awaits(project_root: &Path, event: &EventEnvelope) -> Result<Vec<String>> {
    let mut awaits = load_awaits(project_root)?;
    let matching_ids = awaits
        .iter()
        .filter(|(_, spec)| {
            spec.state == "waiting"
                && event.seq > spec.after_seq
                && spec
                    .topics
                    .iter()
                    .any(|topic| topic_matches(topic, &event.kind))
                && event_key_matches(spec.key.as_deref(), event.key.as_deref())
                && spec
                    .timeout_at
                    .as_deref()
                    .is_none_or(|deadline| event.created_at.as_str() <= deadline)
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &matching_ids {
        fire_await(project_root, &mut awaits, id, event)?;
    }
    if !matching_ids.is_empty() {
        save_awaits(project_root, &awaits)?;
    }
    Ok(matching_ids)
}

fn fire_await(
    project_root: &Path,
    awaits: &mut BTreeMap<String, AwaitSpec>,
    await_id: &str,
    event: &EventEnvelope,
) -> Result<()> {
    let spec = awaits
        .get_mut(await_id)
        .with_context(|| format!("unknown await: {await_id}"))?;
    if spec.state != "waiting" {
        return Ok(());
    }
    spec.state = "fired".to_string();
    spec.matched_event_id = Some(event.id.clone());
    spec.updated_at = now_iso();
    let record = InboxRecord {
        schema: INBOX_SCHEMA.to_string(),
        await_id: spec.id.clone(),
        event: event.clone(),
        delivered_at: now_iso(),
    };
    append_inbox_once(project_root, &spec.owner, &record)
}

fn append_inbox_once(project_root: &Path, owner: &str, record: &InboxRecord) -> Result<()> {
    let path = inbox_path(project_root, owner);
    let existing: Vec<InboxRecord> = read_json_lines(&path)?;
    if existing
        .iter()
        .any(|item| item.await_id == record.await_id && item.event.id == record.event.id)
    {
        return Ok(());
    }
    append_json_line(&path, record)
}

fn topic_matches(topic: &str, kind: &str) -> bool {
    if topic == "*" {
        return true;
    }
    if let Some(prefix) = topic.strip_suffix(".*") {
        return kind == prefix
            || kind
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('.'));
    }
    topic == kind
}

fn event_key_matches(await_key: Option<&str>, event_key: Option<&str>) -> bool {
    await_key.is_none() || await_key == event_key
}

fn validate_event_kind(kind: &str) -> Result<()> {
    if kind.is_empty()
        || kind.len() > 128
        || !kind.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        bail!("invalid event kind {kind:?}");
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        bail!("invalid id {value:?}");
    }
    Ok(())
}

fn validate_owner(owner: &str) -> Result<()> {
    if owner.trim().is_empty()
        || owner.len() > 256
        || owner.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0'))
    {
        bail!("invalid owner");
    }
    Ok(())
}

fn validate_topics(topics: &[String]) -> Result<()> {
    if topics.is_empty() || topics.len() > 32 {
        bail!("await requires between 1 and 32 topics");
    }
    for topic in topics {
        if topic == "*" {
            continue;
        }
        let kind = topic.strip_suffix(".*").unwrap_or(topic);
        validate_event_kind(kind)?;
    }
    Ok(())
}

fn validate_key(key: Option<&str>) -> Result<()> {
    if let Some(key) = key
        && (key.is_empty()
            || key.len() > 256
            || key.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0')))
    {
        bail!("invalid event key");
    }
    Ok(())
}

fn validate_dedupe_key(dedupe_key: Option<&str>) -> Result<()> {
    if let Some(dedupe_key) = dedupe_key
        && (dedupe_key.is_empty()
            || dedupe_key.len() > 256
            || dedupe_key
                .chars()
                .any(|ch| matches!(ch, '\r' | '\n' | '\0')))
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
                .map(|deadline| {
                    deadline
                        .with_timezone(&Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                })
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

fn canonical_project_root(path: PathBuf) -> Result<PathBuf> {
    fs::canonicalize(&path).with_context(|| format!("canonicalize project root {}", path.display()))
}

fn state_dir(project_root: &Path) -> PathBuf {
    project_root
        .join(".codexflow")
        .join("state")
        .join("supervisor")
}

fn inbox_dir(project_root: &Path) -> PathBuf {
    state_dir(project_root).join("inbox")
}

fn endpoint_path(project_root: &Path) -> PathBuf {
    state_dir(project_root).join("endpoint.json")
}

fn events_path(project_root: &Path) -> PathBuf {
    state_dir(project_root).join("events.jsonl")
}

fn awaits_path(project_root: &Path) -> PathBuf {
    state_dir(project_root).join("awaits.json")
}

fn inbox_path(project_root: &Path, owner: &str) -> PathBuf {
    inbox_dir(project_root).join(format!("{}.jsonl", owner_file_stem(owner)))
}

fn owner_file_stem(owner: &str) -> String {
    let visible = owner
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .take(48)
        .collect::<String>();
    let digest = format!("{:x}", Sha256::digest(owner.as_bytes()));
    format!("{visible}-{}", &digest[..24])
}

fn ensure_state_dirs(project_root: &Path) -> Result<()> {
    fs::create_dir_all(inbox_dir(project_root)).context("create CodexFlow supervisor state")
}

fn read_endpoint(project_root: &Path) -> Result<SupervisorEndpoint> {
    let path = endpoint_path(project_root);
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let endpoint: SupervisorEndpoint =
        serde_json::from_str(&data).context("parse supervisor endpoint")?;
    if endpoint.schema != ENDPOINT_SCHEMA {
        bail!("unsupported supervisor endpoint schema {}", endpoint.schema);
    }
    parse_loopback_address(&endpoint.address)?;
    Ok(endpoint)
}

fn load_awaits(project_root: &Path) -> Result<BTreeMap<String, AwaitSpec>> {
    let path = awaits_path(project_root);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

fn save_awaits(project_root: &Path, awaits: &BTreeMap<String, AwaitSpec>) -> Result<()> {
    write_json_atomic(&awaits_path(project_root), awaits)
}

fn last_event_seq(project_root: &Path) -> Result<u64> {
    let events: Vec<EventEnvelope> = read_json_lines(&events_path(project_root))?;
    Ok(events.last().map_or(0, |event| event.seq))
}

fn read_json_lines<T>(path: &Path) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut data = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if !data.is_empty() && data.last() != Some(&b'\n') {
        let tail_start = data
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let tail = &data[tail_start..];
        match serde_json::from_slice::<T>(tail) {
            Ok(_) => {
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(path)
                    .with_context(|| format!("open {} for JSONL tail repair", path.display()))?;
                file.write_all(b"\n")
                    .with_context(|| format!("repair {} trailing newline", path.display()))?;
                file.sync_data()
                    .with_context(|| format!("sync {} after JSONL tail repair", path.display()))?;
                data.push(b'\n');
            }
            Err(err) if err.is_eof() => {
                let file = OpenOptions::new().write(true).open(path).with_context(|| {
                    format!("open {} for JSONL tail truncation", path.display())
                })?;
                file.set_len(tail_start as u64).with_context(|| {
                    format!("truncate incomplete JSONL tail in {}", path.display())
                })?;
                file.sync_data().with_context(|| {
                    format!("sync {} after JSONL tail truncation", path.display())
                })?;
                data.truncate(tail_start);
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context(format!(
                    "parse non-terminated JSONL tail in {}",
                    path.display()
                )));
            }
        }
    }

    let mut values = Vec::new();
    for (index, line) in data.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        values.push(
            serde_json::from_slice(line)
                .with_context(|| format!("parse {} line {}", path.display(), index + 1))?,
        );
    }
    Ok(values)
}

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {
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
    file.sync_data()
        .with_context(|| format!("sync {}", path.display()))?;
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
    file.sync_all()
        .with_context(|| format!("sync {}", tmp.display()))?;
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
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
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

fn with_state_lock<T>(project_root: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = StateLock::acquire(project_root)?;
    f()
}

struct StateLock {
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
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open supervisor state lock {}", path.display()))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => {
                    file.set_len(0).with_context(|| {
                        format!("truncate supervisor state lock {}", path.display())
                    })?;
                    writeln!(file, "pid={} acquired_at={}", std::process::id(), now_iso())
                        .with_context(|| {
                            format!("write supervisor state lock {}", path.display())
                        })?;
                    file.sync_data().with_context(|| {
                        format!("sync supervisor state lock {}", path.display())
                    })?;
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

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn print_value(value: Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_event(kind: &str, key: Option<&str>, seq: u64) -> EventEnvelope {
        EventEnvelope {
            schema: EVENT_SCHEMA.to_string(),
            seq,
            id: format!("ev-{seq}"),
            kind: kind.to_string(),
            key: key.map(str::to_string),
            dedupe_key: None,
            payload: Value::Null,
            created_at: "now".to_string(),
        }
    }

    fn waiting_spec() -> AwaitSpec {
        AwaitSpec {
            schema: AWAIT_SCHEMA.to_string(),
            id: "worker_wait".to_string(),
            owner: "/root".to_string(),
            topics: vec!["agent.*".to_string()],
            key: None,
            after_seq: 7,
            timeout_at: None,
            state: "waiting".to_string(),
            matched_event_id: None,
            created_at: "old".to_string(),
            updated_at: "old".to_string(),
        }
    }

    #[test]
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
        let inbox: Vec<InboxRecord> =
            read_json_lines(&inbox_path(root, "god")).expect("read inbox");
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
        let events: Vec<EventEnvelope> =
            read_json_lines(&events_path(root)).expect("read timeout event");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "timer.elapsed");
        assert_eq!(
            events[0].dedupe_key.as_deref(),
            Some("await-timeout:timeout_wait")
        );
        let inbox: Vec<InboxRecord> =
            read_json_lines(&inbox_path(root, "god")).expect("read timeout inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].await_id, "timeout_wait");
    }

    #[test]
    fn wire_request_rejects_oversized_and_unterminated_frames() {
        let oversized = format!("{}\n", "x".repeat(MAX_WIRE_BYTES));
        assert!(read_wire_request(oversized.as_bytes()).is_err());
        assert!(read_wire_request(br#"{"op":"status"}"#.as_slice()).is_err());
        let valid = read_wire_request(b"{\"op\":\"status\"}\n".as_slice())
            .expect("bounded request")
            .expect("request frame");
        assert_eq!(valid, "{\"op\":\"status\"}\n");
    }

    #[test]
    fn jsonl_reader_repairs_only_incomplete_trailing_records() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let path = temp.path().join("events.jsonl");
        let event = test_event("build.completed", Some("job-1"), 1);
        let mut bytes = serde_json::to_vec(&event).expect("serialize event");
        bytes.push(b'\n');
        bytes.extend_from_slice(br#"{"schema":"codexflow.event.v1""#);
        fs::write(&path, bytes).expect("write torn JSONL");

        let events: Vec<EventEnvelope> = read_json_lines(&path).expect("recover torn tail");
        assert_eq!(events.len(), 1);
        let repaired = fs::read(&path).expect("read repaired JSONL");
        assert!(repaired.ends_with(b"\n"));
        assert_eq!(repaired.iter().filter(|byte| **byte == b'\n').count(), 1);
    }

    #[test]
    fn jsonl_reader_preserves_valid_record_missing_only_newline() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let path = temp.path().join("events.jsonl");
        let event = test_event("build.completed", Some("job-1"), 1);
        fs::write(&path, serde_json::to_vec(&event).expect("serialize event"))
            .expect("write complete JSONL tail");

        let events: Vec<EventEnvelope> = read_json_lines(&path).expect("repair newline");
        assert_eq!(events.len(), 1);
        assert!(
            fs::read(&path)
                .expect("read repaired JSONL")
                .ends_with(b"\n")
        );
    }

    #[test]
    fn pre_deadline_event_wins_after_restart_even_when_deadline_is_past() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        let deadline = (Utc::now() - chrono::Duration::seconds(5))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        register_await_with_timeout(
            root,
            "deadline_wait".to_string(),
            "god".to_string(),
            vec!["build.completed".to_string()],
            Some("job-1".to_string()),
            0,
            Some(deadline.clone()),
        )
        .expect("register timed await");
        let event = EventEnvelope {
            schema: EVENT_SCHEMA.to_string(),
            seq: 1,
            id: "ev-1".to_string(),
            kind: "build.completed".to_string(),
            key: Some("job-1".to_string()),
            dedupe_key: Some("build:job-1".to_string()),
            payload: json!({"status":"ok"}),
            created_at: (Utc::now() - chrono::Duration::seconds(10))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        append_json_line(&events_path(root), &event).expect("append pre-deadline event");

        reconcile_waiting_awaits(root).expect("reconcile waits");
        assert_eq!(process_due_timeouts(root).expect("process timeouts"), 0);
        let awaits = load_awaits(root).expect("load awaits");
        assert_eq!(
            awaits["deadline_wait"].matched_event_id.as_deref(),
            Some("ev-1")
        );
    }

    #[test]
    fn post_deadline_event_cannot_beat_timeout_after_restart() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        let deadline = (Utc::now() - chrono::Duration::seconds(10))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        register_await_with_timeout(
            root,
            "deadline_wait".to_string(),
            "god".to_string(),
            vec!["build.completed".to_string()],
            Some("job-1".to_string()),
            0,
            Some(deadline),
        )
        .expect("register timed await");
        let event = EventEnvelope {
            schema: EVENT_SCHEMA.to_string(),
            seq: 1,
            id: "ev-1".to_string(),
            kind: "build.completed".to_string(),
            key: Some("job-1".to_string()),
            dedupe_key: Some("build:job-1".to_string()),
            payload: json!({"status":"late"}),
            created_at: (Utc::now() - chrono::Duration::seconds(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        append_json_line(&events_path(root), &event).expect("append post-deadline event");

        reconcile_waiting_awaits(root).expect("reconcile waits");
        assert_eq!(
            load_awaits(root).expect("load awaits")["deadline_wait"].state,
            "waiting"
        );
        assert_eq!(process_due_timeouts(root).expect("process timeout"), 1);
        let inbox: Vec<InboxRecord> =
            read_json_lines(&inbox_path(root, "god")).expect("read timeout inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].event.kind, "timer.elapsed");
    }

    #[test]
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

    #[test]
    fn wildcard_topic_matches_expected_event_kinds() {
        assert!(topic_matches("agent.*", "agent.completed"));
        assert!(topic_matches("agent.*", "agent"));
        assert!(!topic_matches("agent.*", "build.completed"));
        assert!(topic_matches("*", "build.completed"));
    }

    #[test]
    fn owner_file_name_is_stable_and_filesystem_safe() {
        let first = owner_file_stem("/root/reviewer:1");
        let second = owner_file_stem("/root/reviewer:1");
        assert_eq!(first, second);
        assert!(!first.contains('/'));
        assert!(!first.contains(':'));
    }

    #[test]
    fn await_registration_is_idempotent_without_comparing_timestamps() {
        let spec = waiting_spec();
        assert!(same_await_registration(
            &spec,
            "/root",
            &["agent.*".to_string()],
            None,
            7
        ));
        assert!(!same_await_registration(
            &spec,
            "/root",
            &["build.*".to_string()],
            None,
            7
        ));
    }

    #[test]
    fn keyed_await_matches_only_the_same_event_key() {
        assert!(event_key_matches(Some("worker-1"), Some("worker-1")));
        assert!(!event_key_matches(Some("worker-1"), Some("worker-2")));
        assert!(!event_key_matches(Some("worker-1"), None));
        assert!(event_key_matches(None, Some("worker-2")));
    }

    #[test]
    fn legacy_awaits_default_to_unkeyed_matching() {
        let value = serde_json::json!({
            "schema": AWAIT_SCHEMA,
            "id": "worker_wait",
            "owner": "/root",
            "topics": ["agent.*"],
            "after_seq": 7,
            "state": "waiting",
            "matched_event_id": null,
            "created_at": "old",
            "updated_at": "old"
        });
        let spec: AwaitSpec = serde_json::from_value(value).expect("legacy await should load");
        assert_eq!(spec.key, None);
    }

    #[test]
    fn keyed_await_reconciles_events_before_and_after_registration() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");

        publish_event(
            root,
            "agent.completed".to_string(),
            Some("worker-1".to_string()),
            Value::Null,
        )
        .expect("publish event");
        let before = register_await(
            root,
            "before".to_string(),
            "/root".to_string(),
            vec!["agent.completed".to_string()],
            Some("worker-1".to_string()),
            0,
        )
        .expect("register await");
        assert_eq!(before["state"], "fired");

        let after = register_await(
            root,
            "after".to_string(),
            "/root".to_string(),
            vec!["agent.completed".to_string()],
            Some("worker-2".to_string()),
            1,
        )
        .expect("register await");
        assert_eq!(after["state"], "waiting");
        let published = publish_event(
            root,
            "agent.completed".to_string(),
            Some("worker-2".to_string()),
            Value::Null,
        )
        .expect("publish event");
        assert_eq!(published["matched_awaits"], json!(["after"]));
    }

    #[test]
    fn keyed_awaits_isolate_wrong_keys_and_suppress_duplicate_delivery() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        register_await(
            root,
            "wait".to_string(),
            "/root".to_string(),
            vec!["agent.completed".to_string()],
            Some("worker-1".to_string()),
            0,
        )
        .expect("register await");

        let wrong = publish_event(
            root,
            "agent.completed".to_string(),
            Some("worker-2".to_string()),
            Value::Null,
        )
        .expect("publish wrong-key event");
        assert_eq!(wrong["matched_awaits"], json!([]));

        let event = test_event("agent.completed", Some("worker-1"), 2);
        let mut awaits = load_awaits(root).expect("load awaits");
        fire_await(root, &mut awaits, "wait", &event).expect("fire await");
        fire_await(root, &mut awaits, "wait", &event).expect("ignore duplicate fire");
        let inbox: Vec<InboxRecord> =
            read_json_lines(&inbox_path(root, "/root")).expect("read inbox");
        assert_eq!(inbox.len(), 1);
    }

    #[test]
    fn dispatched_publish_is_not_replayed_when_response_is_lost() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake supervisor");
        let endpoint = SupervisorEndpoint {
            schema: ENDPOINT_SCHEMA.to_string(),
            address: listener
                .local_addr()
                .expect("read fake address")
                .to_string(),
            pid: std::process::id(),
            started_at: "now".to_string(),
        };
        write_json_atomic(&endpoint_path(root), &endpoint).expect("write fake endpoint");

        let server_root = root.to_path_buf();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept fake supervisor request");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("read fake request");
            let request: WireRequest =
                serde_json::from_str(line.trim_end()).expect("parse fake request");
            execute_request(&server_root, request).expect("commit fake request");
        });

        let result = execute_with_optional_supervisor(
            root,
            WireRequest::Publish {
                kind: "agent.completed".to_string(),
                key: Some("worker-1".to_string()),
                dedupe_key: None,
                payload: Value::Null,
            },
        );
        assert!(
            result.is_err(),
            "lost response must not trigger local replay"
        );
        server.join().expect("join fake supervisor");

        let events: Vec<EventEnvelope> =
            read_json_lines(&events_path(root)).expect("read persisted events");
        assert_eq!(events.len(), 1, "publish must be committed exactly once");
        assert_eq!(events[0].seq, 1);
    }

    #[test]
    fn restart_reconciliation_fires_a_persisted_keyed_await() {
        let temp = tempfile::tempdir().expect("create supervisor state");
        let root = temp.path();
        ensure_state_dirs(root).expect("create state directories");
        let registered = register_await(
            root,
            "restart".to_string(),
            "/root".to_string(),
            vec!["agent.completed".to_string()],
            Some("worker-1".to_string()),
            0,
        )
        .expect("register await");
        assert_eq!(registered["state"], "waiting");

        append_json_line(
            &events_path(root),
            &test_event("agent.completed", Some("worker-1"), 1),
        )
        .expect("persist event");
        reconcile_waiting_awaits(root).expect("reconcile after restart");

        let awaits = load_awaits(root).expect("load reconciled awaits");
        assert_eq!(awaits["restart"].state, "fired");
        assert_eq!(awaits["restart"].matched_event_id.as_deref(), Some("ev-1"));
    }
}
