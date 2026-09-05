from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one hardening target, found {count}: {old[:120]!r}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def replace_region(path: Path, start_marker: str, end_marker: str, replacement: str) -> None:
    text = path.read_text(encoding="utf-8")
    start = text.find(start_marker)
    if start < 0:
        raise SystemExit(f"{path}: missing hardening start marker {start_marker!r}")
    end = text.find(end_marker, start)
    if end < 0:
        raise SystemExit(f"{path}: missing hardening end marker {end_marker!r}")
    path.write_text(text[:start] + replacement + text[end:], encoding="utf-8")


supervisor = Path("codex-rs/cli/src/bin/codexflow-supervisor.rs")

replace_once(
    supervisor,
    "use serde_json::json;\n",
    "use serde_json::json;\nuse sha2::Digest;\nuse sha2::Sha256;\n",
)
replace_once(
    supervisor,
    "use std::collections::BTreeMap;\nuse std::collections::hash_map::DefaultHasher;\n",
    "use std::collections::BTreeMap;\n",
)
replace_once(supervisor, "use std::hash::Hash;\nuse std::hash::Hasher;\n", "")
replace_once(
    supervisor,
    "use std::io::BufReader;\nuse std::io::Write;\n",
    "use std::io::BufReader;\nuse std::io::Read;\nuse std::io::Write;\n",
)
replace_once(
    supervisor,
    "const TIMER_TICK: Duration = Duration::from_millis(50);\n",
    "const TIMER_TICK: Duration = Duration::from_millis(50);\nconst MAX_WIRE_BYTES: usize = 1024 * 1024;\n",
)

replace_region(
    supervisor,
    "fn handle_connection(project_root: &Path, mut stream: TcpStream) -> Result<()> {",
    "\nfn execute_with_optional_supervisor(",
    r'''fn handle_connection(project_root: &Path, mut stream: TcpStream) -> Result<()> {
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
''',
)

replace_once(
    supervisor,
    '''        save_awaits(project_root, &awaits)?;
        Ok(due_ids.len())
''',
    '''        if !due_ids.is_empty() {
            save_awaits(project_root, &awaits)?;
        }
        Ok(due_ids.len())
''',
)

replace_once(
    supervisor,
    '''    save_awaits(project_root, &awaits)?;
    Ok(matching_ids)
}

fn fire_await(
''',
    '''    if !matching_ids.is_empty() {
        save_awaits(project_root, &awaits)?;
    }
    Ok(matching_ids)
}

fn fire_await(
''',
)

replace_once(
    supervisor,
    '''            && event_key_matches(spec.key.as_deref(), event.key.as_deref())
    }) {
''',
    '''            && event_key_matches(spec.key.as_deref(), event.key.as_deref())
            && spec
                .timeout_at
                .as_deref()
                .is_none_or(|deadline| event.created_at.as_str() <= deadline)
    }) {
''',
)
replace_once(
    supervisor,
    '''                && event_key_matches(spec.key.as_deref(), event.key.as_deref())
        })
''',
    '''                && event_key_matches(spec.key.as_deref(), event.key.as_deref())
                && spec
                    .timeout_at
                    .as_deref()
                    .is_none_or(|deadline| event.created_at.as_str() <= deadline)
        })
''',
)

replace_region(
    supervisor,
    "fn read_json_lines<T>(path: &Path) -> Result<Vec<T>>",
    "\nfn append_json_line<T: Serialize>",
    r'''fn read_json_lines<T>(path: &Path) -> Result<Vec<T>>
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
                let file = OpenOptions::new()
                    .write(true)
                    .open(path)
                    .with_context(|| format!("open {} for JSONL tail truncation", path.display()))?;
                file.set_len(tail_start as u64)
                    .with_context(|| format!("truncate incomplete JSONL tail in {}", path.display()))?;
                file.sync_data()
                    .with_context(|| format!("sync {} after JSONL tail truncation", path.display()))?;
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
''',
)

replace_region(
    supervisor,
    "fn owner_file_stem(owner: &str) -> String {",
    "\nfn ensure_state_dirs(",
    r'''fn owner_file_stem(owner: &str) -> String {
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
''',
)

cargo = Path("codex-rs/cli/Cargo.toml")
replace_once(
    cargo,
    "serde_json = { workspace = true }\n",
    "serde_json = { workspace = true }\nsha2 = { workspace = true }\n",
)

text = supervisor.read_text(encoding="utf-8")
marker = "    #[test]\n    fn wildcard_topic_matches_expected_event_kinds() {"
idx = text.find(marker)
if idx < 0:
    raise SystemExit("supervisor: missing hardening test insertion marker")
new_tests = r'''    #[test]
    fn wire_request_rejects_oversized_and_unterminated_frames() {
        let oversized = format!("{}\n", "x".repeat(MAX_WIRE_BYTES));
        assert!(read_wire_request(oversized.as_bytes()).is_err());
        assert!(read_wire_request(br#"{"op":"status"}"#).is_err());
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
        assert!(fs::read(&path).expect("read repaired JSONL").ends_with(b"\n"));
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
        assert_eq!(awaits["deadline_wait"].matched_event_id.as_deref(), Some("ev-1"));
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
        assert_eq!(load_awaits(root).expect("load awaits")["deadline_wait"].state, "waiting");
        assert_eq!(process_due_timeouts(root).expect("process timeout"), 1);
        let inbox: Vec<InboxRecord> =
            read_json_lines(&inbox_path(root, "god")).expect("read timeout inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].event.kind, "timer.elapsed");
    }

'''
supervisor.write_text(text[:idx] + new_tests + text[idx:], encoding="utf-8")
