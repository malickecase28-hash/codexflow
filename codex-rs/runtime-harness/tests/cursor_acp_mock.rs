#![cfg(unix)]

use codex_runtime_harness::CursorAcpBackend;
use codex_runtime_harness::CursorAcpConfig;
use codex_runtime_harness::CursorAcpError;
use codex_runtime_harness::PermissionOutcome;
use codex_runtime_harness::PermissionRequest;
use codex_runtime_harness::RuntimeEvent;
use codex_runtime_harness::RuntimeEventSink;
use codex_runtime_harness::RuntimeInteractionHandler;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

const MOCK_AGENT: &str = r#"#!/usr/bin/env python3
import json
import sys


def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()


for raw in sys.stdin:
    message = json.loads(raw)
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}

    if method == "initialize":
        send({"jsonrpc":"2.0","id":request_id,"result":{"protocolVersion":1}})
    elif method == "authenticate":
        if params.get("methodId") != "cursor_login":
            send({"jsonrpc":"2.0","id":request_id,"error":{"code":-32000,"message":"wrong auth method"}})
        else:
            send({"jsonrpc":"2.0","id":request_id,"result":{}})
    elif method == "session/new":
        send({"jsonrpc":"2.0","id":request_id,"result":{"sessionId":"mock-session"}})
    elif method == "session/load":
        send({"jsonrpc":"2.0","id":request_id,"result":{}})
    elif method == "session/prompt":
        prompt = params.get("prompt") or []
        text = prompt[0].get("text", "") if prompt else ""
        if text == "crash":
            sys.exit(0)
        send({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello from cursor"}}}})
        send({"jsonrpc":"2.0","id":900,"method":"session/request_permission","params":{"sessionId":"mock-session","toolCall":{"toolCallId":"tool-1"},"options":[{"optionId":"allow-once","name":"Allow once"},{"optionId":"reject-once","name":"Reject"}]}})
        permission_reply = json.loads(sys.stdin.readline())
        selected = (((permission_reply.get("result") or {}).get("outcome") or {}).get("optionId"))
        if selected != "allow-once":
            send({"jsonrpc":"2.0","id":request_id,"error":{"code":-32001,"message":"permission mapping failed"}})
            continue
        send({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-session","update":{"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"mock tool"}}})
        send({"jsonrpc":"2.0","id":request_id,"result":{"stopReason":"end_turn"}})
    elif method == "session/cancel":
        pass
"#;

#[derive(Default)]
struct Collector(Mutex<Vec<RuntimeEvent>>);

impl RuntimeEventSink for Collector {
    fn emit(&self, event: RuntimeEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct AllowOnce;

impl RuntimeInteractionHandler for AllowOnce {
    fn decide_permission(&self, _request: &PermissionRequest) -> PermissionOutcome {
        PermissionOutcome::AllowOnce
    }
}

fn write_mock_agent(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("mock-agent");
    fs::write(&path, MOCK_AGENT).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[tokio::test]
async fn cursor_acp_round_trips_stream_tool_and_permission_events() {
    let temp = tempfile::tempdir().unwrap();
    let executable = write_mock_agent(temp.path());
    let backend = CursorAcpBackend::connect(CursorAcpConfig {
        executable: Some(executable),
        process_cwd: Some(temp.path().to_path_buf()),
    })
    .await
    .unwrap();

    let session = backend.new_session(temp.path()).await.unwrap();
    assert_eq!(session.0, "mock-session");

    let collector = Arc::new(Collector::default());
    backend
        .load_session(&session, temp.path(), collector.clone())
        .await
        .unwrap();
    let stop_reason = backend
        .prompt(&session, "hello", collector.clone(), Arc::new(AllowOnce))
        .await
        .unwrap();
    assert_eq!(stop_reason.as_deref(), Some("end_turn"));

    let events = collector.0.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeEvent::AgentMessageChunk { text } if text == "hello from cursor"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeEvent::PermissionRequest { request }
            if request.session_id.as_deref() == Some("mock-session")
                && request.options.iter().any(|option| option.option_id == "allow-once")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeEvent::ToolCall { raw }
            if raw.get("toolCallId").and_then(Value::as_str) == Some("tool-1")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeEvent::Completed { stop_reason }
            if stop_reason.as_deref() == Some("end_turn")
    )));
    drop(events);

    backend.cancel(&session).await.unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test]
async fn cursor_acp_transport_crash_requires_and_supports_reconnect() {
    let temp = tempfile::tempdir().unwrap();
    let executable = write_mock_agent(temp.path());
    let backend = CursorAcpBackend::connect(CursorAcpConfig {
        executable: Some(executable),
        process_cwd: Some(temp.path().to_path_buf()),
    })
    .await
    .unwrap();
    let session = backend.new_session(temp.path()).await.unwrap();
    let collector = Arc::new(Collector::default());

    let error = backend
        .prompt(&session, "crash", collector.clone(), Arc::new(AllowOnce))
        .await
        .unwrap_err();
    assert!(matches!(error, CursorAcpError::UnexpectedEof(method) if method == "session/prompt"));

    assert!(matches!(
        backend.new_session(temp.path()).await,
        Err(CursorAcpError::ConnectionUnavailable)
    ));

    backend.reconnect().await.unwrap();
    let recovered = backend.new_session(temp.path()).await.unwrap();
    assert_eq!(recovered.0, "mock-session");
    backend.shutdown().await.unwrap();
}
