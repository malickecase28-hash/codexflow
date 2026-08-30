use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::fs::OpenOptions;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const ENDPOINT_SCHEMA: &str = "codexflow.supervisor.endpoint.v1";
const EVENT_SCHEMA: &str = "codexflow.event.v1";
const AWAIT_SCHEMA: &str = "codexflow.await.v1";
const INBOX_SCHEMA: &str = "codexflow.inbox.v1";
const LOCK_STALE_AFTER: Duration = Duration::from_secs(120);
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(8);

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
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
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
    payload: Value,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AwaitSpec {
    schema: String,
    id: String,
    owner: String,
    topics: Vec<String>,
    after_seq: u64,
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
        payload: Value,
    },
    RegisterAwait {
        id: String,
        owner: String,
        topics: Vec<String>,
        after_seq: u64,
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
            payload,
        } => {
            let root = canonical_project_root(project_root)?;
            validate_event_kind(&kind)?;
            let payload: Value =
                serde_json::from_str(&payload).context("parse --payload as JSON")?;
            print_value(execute_with_optional_supervisor(
                &root,
                WireRequest::Publish { kind, key, payload },
            )?)
        }
        Command::Await {
            project_root,
            id,
            owner,
            topics,
            after_seq,
        } => {
            let root = canonical_project_root(project_root)?;
            print_value(execute_with_optional_supervisor(
                &root,
                WireRequest::RegisterAwait {
                    id,
                    owner,
                    topics,
                    after_seq,
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
            print_value(execute_with_optional_supervisor(&root, WireRequest::Status)?)
        }
    }
}

fn run_supervisor(project_root: &Path, bind: &str) -> Result<()> {
    ensure_state_dirs(project_root)?;
    reconcile_waiting_awaits(project_root)?;
    let listener = TcpListener::bind(bind).with_context(|| format!("bind supervisor to {bind}"))?;
    let address = listener.local_addr().context("read supervisor address")?;
    let endpoint = SupervisorEndpoint {
        schema: ENDPOINT_SCHEMA.to_string(),
        address: address.to_string(),
        pid: std::process::id(),
        started_at: now_iso(),
    };
    write_json_atomic(&endpoint_path(project_root), &endpoint)?;
    println!("{}", serde_json::to_string_pretty(&endpoint)?);

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                if let Err(err) = handle_connection(project_root, stream) {
                    eprintln!("CodexFlow supervisor request failed: {err:#}");
                }
            }
            Err(err) => eprintln!("CodexFlow supervisor accept failed: {err}"),
        }
    }
    Ok(())
}

fn handle_connection(project_root: &Path, mut stream: TcpStream) -> Result<()> {
    let read_stream = stream.try_clone().context("clone supervisor socket")?;
    let mut reader = BufReader::new(read_stream);
    let mut request_line = String::new();
    let bytes = reader
        .read_line(&mut request_line)
        .context("read supervisor request")?;
    if bytes == 0 {
        return Ok(());
    }

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

fn execute_with_optional_supervisor(project_root: &Path, request: WireRequest) -> Result<Value> {
    if let Ok(endpoint) = read_endpoint(project_root)
        && let Ok(value) = send_request(&endpoint, &request)
    {
        return Ok(value);
    }
    execute_request(project_root, request)
}

fn send_request(endpoint: &SupervisorEndpoint, request: &WireRequest) -> Result<Value> {
    if endpoint.schema != ENDPOINT_SCHEMA {
        bail!("unsupported supervisor endpoint schema {}", endpoint.schema);
    }
    let mut stream = TcpStream::connect(&endpoint.address)
        .with_context(|| format!("connect to supervisor {}", endpoint.address))?;
    writeln!(stream, "{}", serde_json::to_string(request)?)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("read supervisor response")?;
    let response: WireResponse =
        serde_json::from_str(line.trim_end()).context("parse supervisor response")?;
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "supervisor request failed".to_string())
        );
    }
    Ok(response.value.unwrap_or(Value::Null))
}

fn execute_request(project_root: &Path, request: WireRequest) -> Result<Value> {
    ensure_state_dirs(project_root)?;
    match request {
        WireRequest::Publish { kind, key, payload } => {
            validate_event_kind(&kind)?;
            publish_event(project_root, kind, key, payload)
        }
        WireRequest::RegisterAwait {
            id,
            owner,
            topics,
            after_seq,
        } => register_await(project_root, id, owner, topics, after_seq),
        WireRequest::Inbox { owner, clear } => read_inbox(project_root, &owner, clear),
        WireRequest::Status => supervisor_status(project_root),
    }
}

fn publish_event(
    project_root: &Path,
    kind: String,
    key: Option<String>,
    payload: Value,
) -> Result<Value> {
    with_state_lock(project_root, || {
        let seq = last_event_seq(project_root)?.saturating_add(1);
        let event = EventEnvelope {
            schema: EVENT_SCHEMA.to_string(),
            seq,
            id: format!("ev-{seq}"),
            kind,
            key,
            payload,
            created_at: now_iso(),
        };
        append_json_line(&events_path(project_root), &event)?;
        let matched = resolve_event_against_awaits(project_root, &event)?;
        Ok(json!({ "event": event, "matched_awaits": matched }))
    })
}

fn register_await(
    project_root: &Path,
    id: String,
    owner: String,
    topics: Vec<String>,
    after_seq: u64,
) -> Result<Value> {
    validate_id(&id)?;
    validate_owner(&owner)?;
    validate_topics(&topics)?;
    with_state_lock(project_root, || {
        let mut awaits = load_awaits(project_root)?;
        if let Some(existing) = awaits.get(&id) {
            if same_await_registration(existing, &owner, &topics, after_seq) {
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
                after_seq,
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

fn same_await_registration(
    existing: &AwaitSpec,
    owner: &str,
    topics: &[String],
    after_seq: u64,
) -> bool {
    existing.owner == owner && existing.topics == topics && existing.after_seq == after_seq
}

fn read_inbox(project_root: &Path, owner: &str, clear: bool) -> Result<Value> {
    validate_owner(owner)?;
    with_state_lock(project_root, || {
        let path = inbox_path(project_root, owner);
        let records: Vec<InboxRecord> = read_json_lines(&path)?;
        if clear && path.exists() {
            fs::write(&path, b"").with_context(|| format!("clear {}", path.display()))?;
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
    let fired = awaits
        .values()
        .filter(|spec| spec.state == "fired")
        .count();
    Ok(json!({
        "project_root": project_root,
        "endpoint": endpoint,
        "last_event_seq": last_event_seq(project_root)?,
        "awaits": { "waiting": waiting, "fired": fired, "total": awaits.len() }
    }))
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
    }) {
        fire_await(project_root, awaits, await_id, &event)?;
    }
    Ok(())
}

fn resolve_event_against_awaits(
    project_root: &Path,
    event: &EventEnvelope,
) -> Result<Vec<String>> {
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
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &matching_ids {
        fire_await(project_root, &mut awaits, id, event)?;
    }
    save_awaits(project_root, &awaits)?;
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

fn validate_event_kind(kind: &str) -> Result<()> {
    if kind.is_empty()
        || kind.len() > 128
        || !kind.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        bail!("invalid event kind {kind:?}");
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 96
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/')
        })
    {
        bail!("invalid id {value:?}");
    }
    Ok(())
}

fn validate_owner(owner: &str) -> Result<()> {
    if owner.trim().is_empty()
        || owner.len() > 256
        || owner
            .chars()
            .any(|ch| matches!(ch, '\r' | '\n' | '\0'))
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

fn canonical_project_root(path: PathBuf) -> Result<PathBuf> {
    fs::canonicalize(&path)
        .with_context(|| format!("canonicalize project root {}", path.display()))
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
    let mut hasher = DefaultHasher::new();
    owner.hash(&mut hasher);
    format!("{visible}-{:016x}", hasher.finish())
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
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut values = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("read {} line {}", path.display(), index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        values.push(
            serde_json::from_str(&line)
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
    Ok(())
}

fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    if cfg!(windows) && path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

fn with_state_lock<T>(project_root: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = StateLock::acquire(project_root)?;
    f()
}

struct StateLock {
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

    fn waiting_spec() -> AwaitSpec {
        AwaitSpec {
            schema: AWAIT_SCHEMA.to_string(),
            id: "worker_wait".to_string(),
            owner: "/root".to_string(),
            topics: vec!["agent.*".to_string()],
            after_seq: 7,
            state: "waiting".to_string(),
            matched_event_id: None,
            created_at: "old".to_string(),
            updated_at: "old".to_string(),
        }
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
            7
        ));
        assert!(!same_await_registration(
            &spec,
            "/root",
            &["build.*".to_string()],
            7
        ));
    }
}
