use crate::types::PermissionOption;
use crate::types::PermissionOutcome;
use crate::types::PermissionRequest;
use crate::types::RuntimeEvent;
use crate::types::RuntimeEventSink;
use crate::types::RuntimeInteractionHandler;
use crate::types::RuntimeSessionId;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum CursorAcpError {
    #[error("failed to launch Cursor ACP agent: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("Cursor ACP child did not expose {0}")]
    MissingPipe(&'static str),
    #[error("Cursor ACP transport failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cursor ACP emitted invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Cursor ACP closed stdout before request {0} completed")]
    UnexpectedEof(String),
    #[error("Cursor ACP request {method} failed: {error}")]
    Rpc { method: String, error: Value },
    #[error("Cursor ACP response to {0} omitted the required field '{1}'")]
    MissingField(String, &'static str),
}

#[derive(Debug, Clone, Default)]
pub struct CursorAcpConfig {
    /// Explicit ACP executable. If omitted, CODEX_CURSOR_AGENT is honored,
    /// then `agent`, then legacy `cursor-agent` are attempted.
    pub executable: Option<PathBuf>,
    pub process_cwd: Option<PathBuf>,
}

pub struct CursorAcpBackend {
    connection: Mutex<AcpConnection>,
}

struct AcpConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl CursorAcpBackend {
    pub async fn connect(config: CursorAcpConfig) -> Result<Self, CursorAcpError> {
        let mut connection = AcpConnection::spawn(config).await?;
        connection.initialize().await?;
        connection.authenticate().await?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub async fn new_session(&self, cwd: &Path) -> Result<RuntimeSessionId, CursorAcpError> {
        let mut connection = self.connection.lock().await;
        let result = connection
            .request("session/new", json!({ "cwd": cwd, "mcpServers": [] }), None, None)
            .await?;
        session_id_from_result("session/new", result)
    }

    pub async fn load_session(
        &self,
        session_id: &RuntimeSessionId,
        cwd: &Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> Result<(), CursorAcpError> {
        let mut connection = self.connection.lock().await;
        connection
            .request(
                "session/load",
                json!({ "sessionId": session_id.0, "cwd": cwd, "mcpServers": [] }),
                Some(sink),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn prompt(
        &self,
        session_id: &RuntimeSessionId,
        text: &str,
        sink: Arc<dyn RuntimeEventSink>,
        interactions: Arc<dyn RuntimeInteractionHandler>,
    ) -> Result<Option<String>, CursorAcpError> {
        let mut connection = self.connection.lock().await;
        let result = connection
            .request(
                "session/prompt",
                json!({
                    "sessionId": session_id.0,
                    "prompt": [{ "type": "text", "text": text }]
                }),
                Some(Arc::clone(&sink)),
                Some(interactions),
            )
            .await?;
        let stop_reason = result
            .get("stopReason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        sink.emit(RuntimeEvent::Completed {
            stop_reason: stop_reason.clone(),
        });
        Ok(stop_reason)
    }

    /// ACP defines cancellation as a client notification, not a request.
    pub async fn cancel(&self, session_id: &RuntimeSessionId) -> Result<(), CursorAcpError> {
        let mut connection = self.connection.lock().await;
        connection
            .notify("session/cancel", json!({ "sessionId": session_id.0 }))
            .await
    }

    pub async fn shutdown(&self) -> Result<(), CursorAcpError> {
        let mut connection = self.connection.lock().await;
        let _ = connection.stdin.shutdown().await;
        if connection.child.try_wait()?.is_none() {
            connection.child.kill().await?;
        }
        Ok(())
    }
}

impl AcpConnection {
    async fn spawn(config: CursorAcpConfig) -> Result<Self, CursorAcpError> {
        let candidates: Vec<PathBuf> = if let Some(executable) = config.executable {
            vec![executable]
        } else if let Some(executable) = std::env::var_os("CODEX_CURSOR_AGENT") {
            vec![PathBuf::from(executable)]
        } else {
            vec![PathBuf::from("agent"), PathBuf::from("cursor-agent")]
        };

        let mut last_not_found = None;
        for executable in candidates {
            let mut command = Command::new(&executable);
            command.arg("acp");
            if let Some(cwd) = config.process_cwd.as_deref() {
                command.current_dir(cwd);
            }
            command.stdin(Stdio::piped());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::inherit());
            match command.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take().ok_or(CursorAcpError::MissingPipe("stdin"))?;
                    let stdout = child.stdout.take().ok_or(CursorAcpError::MissingPipe("stdout"))?;
                    return Ok(Self {
                        child,
                        stdin,
                        stdout: BufReader::new(stdout),
                        next_id: 1,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    last_not_found = Some(error);
                }
                Err(error) => return Err(CursorAcpError::Spawn(error)),
            }
        }

        Err(CursorAcpError::Spawn(last_not_found.unwrap_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no Cursor agent executable found")
        })))
    }

    async fn initialize(&mut self) -> Result<(), CursorAcpError> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                },
                "clientInfo": { "name": "codexflow", "version": env!("CARGO_PKG_VERSION") }
            }),
            None,
            None,
        )
        .await?;
        Ok(())
    }

    async fn authenticate(&mut self) -> Result<(), CursorAcpError> {
        self.request(
            "authenticate",
            json!({ "methodId": "cursor_login" }),
            None,
            None,
        )
        .await?;
        Ok(())
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), CursorAcpError> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        sink: Option<Arc<dyn RuntimeEventSink>>,
        interactions: Option<Arc<dyn RuntimeInteractionHandler>>,
    ) -> Result<Value, CursorAcpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;

        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).await? == 0 {
                return Err(CursorAcpError::UnexpectedEof(method.to_string()));
            }
            let message: Value = serde_json::from_str(line.trim_end())?;

            if message.get("id") == Some(&Value::from(id))
                && (message.get("result").is_some() || message.get("error").is_some())
            {
                if let Some(error) = message.get("error") {
                    return Err(CursorAcpError::Rpc {
                        method: method.to_string(),
                        error: error.clone(),
                    });
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }

            self.handle_server_message(message, sink.as_deref(), interactions.as_deref())
                .await?;
        }
    }

    async fn handle_server_message(
        &mut self,
        message: Value,
        sink: Option<&dyn RuntimeEventSink>,
        interactions: Option<&dyn RuntimeInteractionHandler>,
    ) -> Result<(), CursorAcpError> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(());
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "session/update" => {
                if let Some(sink) = sink {
                    sink.emit(normalize_session_update(&params));
                }
            }
            "session/request_permission" => {
                let Some(request_id) = message.get("id").cloned() else {
                    return Ok(());
                };
                let request = parse_permission_request(request_id.clone(), params.clone());
                if let Some(sink) = sink {
                    sink.emit(RuntimeEvent::PermissionRequest {
                        request: request.clone(),
                    });
                }
                let outcome = interactions
                    .map(|handler| handler.decide_permission(&request))
                    .unwrap_or(PermissionOutcome::RejectOnce);
                self.write_message(&json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "outcome": {
                            "outcome": "selected",
                            "optionId": outcome.option_id()
                        }
                    }
                }))
                .await?;
            }
            "cursor/ask_question" | "cursor/create_plan" => {
                if let Some(sink) = sink {
                    sink.emit(RuntimeEvent::CursorExtension {
                        method: method.to_string(),
                        params: params.clone(),
                    });
                }
                if let Some(request_id) = message.get("id").cloned() {
                    if let Some(result) = interactions
                        .and_then(|handler| handler.handle_cursor_extension(method, &params))
                    {
                        self.write_message(&json!({
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "result": result
                        }))
                        .await?;
                    } else {
                        self.write_message(&json!({
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "error": {
                                "code": -32601,
                                "message": "Cursor extension is not handled by this client"
                            }
                        }))
                        .await?;
                    }
                }
            }
            method if method.starts_with("cursor/") => {
                if let Some(sink) = sink {
                    sink.emit(RuntimeEvent::CursorExtension {
                        method: method.to_string(),
                        params,
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn write_message(&mut self, message: &Value) -> Result<(), CursorAcpError> {
        let mut encoded = serde_json::to_vec(message)?;
        encoded.push(b'\n');
        self.stdin.write_all(&encoded).await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

fn session_id_from_result(
    method: &str,
    result: Value,
) -> Result<RuntimeSessionId, CursorAcpError> {
    result
        .get("sessionId")
        .and_then(Value::as_str)
        .map(|id| RuntimeSessionId(id.to_string()))
        .ok_or_else(|| CursorAcpError::MissingField(method.to_string(), "sessionId"))
}

pub fn normalize_session_update(params: &Value) -> RuntimeEvent {
    let update = params.get("update").cloned().unwrap_or(Value::Null);
    let update_type = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match update_type {
        "agent_message_chunk" => RuntimeEvent::AgentMessageChunk {
            text: update
                .get("content")
                .and_then(|content| content.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "tool_call" => RuntimeEvent::ToolCall { raw: update },
        "tool_call_update" => RuntimeEvent::ToolCallUpdate { raw: update },
        "plan" => RuntimeEvent::Plan { raw: update },
        "usage_update" => RuntimeEvent::UsageUpdate { raw: update },
        _ => RuntimeEvent::SessionUpdate {
            update_type: update_type.to_string(),
            raw: update,
        },
    }
}

pub fn parse_permission_request(request_id: Value, params: Value) -> PermissionRequest {
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let option_id = option.get("optionId")?.as_str()?.to_string();
            Some(PermissionOption {
                option_id,
                name: option.get("name").and_then(Value::as_str).map(str::to_string),
                kind: option.get("kind").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect();
    PermissionRequest {
        request_id,
        session_id: params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
        tool_call: params.get("toolCall").cloned(),
        options,
        raw_params: params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_agent_message_chunk() {
        let event = normalize_session_update(&json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hello" }
            }
        }));
        assert_eq!(event, RuntimeEvent::AgentMessageChunk { text: "hello".into() });
    }

    #[test]
    fn permission_parser_preserves_explicit_option_ids() {
        let request = parse_permission_request(
            Value::from(7),
            json!({
                "sessionId": "s1",
                "options": [
                    { "optionId": "allow-once", "name": "Allow once" },
                    { "optionId": "reject-once", "name": "Reject" }
                ],
                "toolCall": { "toolCallId": "tool-1" }
            }),
        );
        assert_eq!(request.options.len(), 2);
        assert_eq!(request.options[0].option_id, "allow-once");
        assert_eq!(request.session_id.as_deref(), Some("s1"));
    }
}
