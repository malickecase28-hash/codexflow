from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one patch anchor, found {count}")
    file.write_text(text.replace(old, new, 1))


cursor = "codex-rs/runtime-harness/src/cursor_acp.rs"
replace_once(
    cursor,
    '''    #[error("Cursor ACP control operation failed: {0}")]
    Control(String),
''',
    '''    #[error("Cursor ACP control operation failed: {0}")]
    Control(String),
    #[error("Cursor ACP session did not expose a model selector for requested model '{0}'")]
    ModelConfigUnavailable(String),
    #[error("Cursor ACP model '{0}' is not available in this session")]
    ModelUnavailable(String),
''',
)

replace_once(
    cursor,
    '''    pub async fn new_session(&self, cwd: &Path) -> Result<RuntimeSessionId, CursorAcpError> {
        let (_permit, mut connection) = self.take_connection().await?;
        let result = connection
            .request(
                "session/new",
                json!({ "cwd": cwd, "mcpServers": [] }),
                None,
                None,
            )
            .await;
        let result = self.complete_operation(connection, result).await?;
        session_id_from_result("session/new", result)
    }
''',
    '''    pub async fn new_session(&self, cwd: &Path) -> Result<RuntimeSessionId, CursorAcpError> {
        self.new_session_inner(cwd, None).await
    }

    pub async fn new_session_with_model(
        &self,
        cwd: &Path,
        model: &str,
    ) -> Result<RuntimeSessionId, CursorAcpError> {
        self.new_session_inner(cwd, Some(model)).await
    }

    async fn new_session_inner(
        &self,
        cwd: &Path,
        model: Option<&str>,
    ) -> Result<RuntimeSessionId, CursorAcpError> {
        let (_permit, mut connection) = self.take_connection().await?;
        let result = connection
            .request(
                "session/new",
                json!({ "cwd": cwd, "mcpServers": [] }),
                None,
                None,
            )
            .await;
        let result = match result {
            Ok(result) => {
                let session_id = session_id_from_result("session/new", result.clone())?;
                if let Some(model) = model {
                    connection
                        .apply_session_model(&session_id, &result, model)
                        .await?;
                }
                Ok((session_id, result))
            }
            Err(error) => Err(error),
        };
        let (session_id, _result) = self.complete_operation(connection, result).await?;
        Ok(session_id)
    }
''',
)

replace_once(
    cursor,
    '''    pub async fn load_session(
        &self,
        session_id: &RuntimeSessionId,
        cwd: &Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> Result<(), CursorAcpError> {
        let (_permit, mut connection) = self.take_connection().await?;
        let result = connection
            .request(
                "session/load",
                json!({ "sessionId": session_id.0, "cwd": cwd, "mcpServers": [] }),
                Some(sink),
                None,
            )
            .await;
        self.complete_operation(connection, result).await?;
        Ok(())
    }
''',
    '''    pub async fn load_session(
        &self,
        session_id: &RuntimeSessionId,
        cwd: &Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> Result<(), CursorAcpError> {
        self.load_session_inner(session_id, cwd, None, sink).await
    }

    pub async fn load_session_with_model(
        &self,
        session_id: &RuntimeSessionId,
        cwd: &Path,
        model: &str,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> Result<(), CursorAcpError> {
        self.load_session_inner(session_id, cwd, Some(model), sink)
            .await
    }

    async fn load_session_inner(
        &self,
        session_id: &RuntimeSessionId,
        cwd: &Path,
        model: Option<&str>,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> Result<(), CursorAcpError> {
        let (_permit, mut connection) = self.take_connection().await?;
        let result = connection
            .request(
                "session/load",
                json!({ "sessionId": session_id.0, "cwd": cwd, "mcpServers": [] }),
                Some(sink),
                None,
            )
            .await;
        let result = match result {
            Ok(result) => {
                if let Some(model) = model {
                    connection
                        .apply_session_model(session_id, &result, model)
                        .await?;
                }
                Ok(result)
            }
            Err(error) => Err(error),
        };
        self.complete_operation(connection, result).await?;
        Ok(())
    }
''',
)

replace_once(
    cursor,
    '''    async fn complete_operation<T>(
        &self,
        connection: AcpConnection,
        result: Result<T, CursorAcpError>,
    ) -> Result<T, CursorAcpError> {
        let fatal = matches!(
            &result,
            Err(CursorAcpError::Io(_)
                | CursorAcpError::Json(_)
                | CursorAcpError::UnexpectedEof(_)
                | CursorAcpError::ConnectionUnavailable)
        );
        if !fatal {
            let mut slot = self.connection.lock().await;
            *slot = Some(connection);
        }
        result
    }
''',
    '''    async fn complete_operation<T>(
        &self,
        mut connection: AcpConnection,
        result: Result<T, CursorAcpError>,
    ) -> Result<T, CursorAcpError> {
        let fatal = matches!(
            &result,
            Err(CursorAcpError::Io(_)
                | CursorAcpError::Json(_)
                | CursorAcpError::UnexpectedEof(_)
                | CursorAcpError::ConnectionUnavailable)
        );
        if fatal {
            let _ = connection.terminate().await;
        } else {
            let mut slot = self.connection.lock().await;
            *slot = Some(connection);
        }
        result
    }
''',
)

replace_once(
    cursor,
    '''            command.stdin(Stdio::piped());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::inherit());
''',
    '''            command.stdin(Stdio::piped());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::inherit());
            command.kill_on_drop(true);
''',
)

replace_once(
    cursor,
    '''    async fn notify(&mut self, method: &str, params: Value) -> Result<(), CursorAcpError> {
''',
    '''    async fn apply_session_model(
        &mut self,
        session_id: &RuntimeSessionId,
        session_result: &Value,
        model: &str,
    ) -> Result<(), CursorAcpError> {
        let config = find_model_config(session_result, model)
            .ok_or_else(|| CursorAcpError::ModelConfigUnavailable(model.to_string()))?;
        if !config_option_contains_value(config, model) {
            return Err(CursorAcpError::ModelUnavailable(model.to_string()));
        }
        if config.get("currentValue").and_then(Value::as_str) == Some(model) {
            return Ok(());
        }
        let config_id = config
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| CursorAcpError::ModelConfigUnavailable(model.to_string()))?;
        self.request(
            "session/set_config_option",
            json!({
                "sessionId": session_id.0,
                "configId": config_id,
                "value": model
            }),
            None,
            None,
        )
        .await?;
        Ok(())
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), CursorAcpError> {
''',
)

replace_once(
    cursor,
    '''fn session_id_from_result(method: &str, result: Value) -> Result<RuntimeSessionId, CursorAcpError> {
    result
        .get("sessionId")
        .and_then(Value::as_str)
        .map(|id| RuntimeSessionId(id.to_string()))
        .ok_or_else(|| CursorAcpError::MissingField(method.to_string(), "sessionId"))
}

pub fn normalize_session_update(params: &Value) -> RuntimeEvent {
''',
    '''fn session_id_from_result(method: &str, result: Value) -> Result<RuntimeSessionId, CursorAcpError> {
    result
        .get("sessionId")
        .and_then(Value::as_str)
        .map(|id| RuntimeSessionId(id.to_string()))
        .ok_or_else(|| CursorAcpError::MissingField(method.to_string(), "sessionId"))
}

fn find_model_config<'a>(session_result: &'a Value, model: &str) -> Option<&'a Value> {
    let configs = session_result.get("configOptions")?.as_array()?;
    configs
        .iter()
        .find(|config| config.get("category").and_then(Value::as_str) == Some("model"))
        .or_else(|| {
            configs.iter().find(|config| {
                config
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| matches!(id.to_ascii_lowercase().as_str(), "model" | "models"))
                    || config
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case("model"))
            })
        })
        .or_else(|| {
            let mut matching = configs
                .iter()
                .filter(|config| config_option_contains_value(config, model));
            let first = matching.next()?;
            matching.next().is_none().then_some(first)
        })
}

fn config_option_contains_value(config: &Value, target: &str) -> bool {
    match config {
        Value::Array(values) => values
            .iter()
            .any(|value| config_option_contains_value(value, target)),
        Value::Object(map) => {
            map.get("value").and_then(Value::as_str) == Some(target)
                || map
                    .values()
                    .any(|value| config_option_contains_value(value, target))
        }
        _ => false,
    }
}

pub fn normalize_session_update(params: &Value) -> RuntimeEvent {
''',
)

router = "codex-rs/runtime-harness/src/router.rs"
replace_once(
    router,
    '''    fn create_session<'a>(&'a self, cwd: &'a Path) -> BackendFuture<'a, RuntimeSessionId>;

    fn load_session<'a>(
''',
    '''    fn create_session<'a>(&'a self, cwd: &'a Path) -> BackendFuture<'a, RuntimeSessionId>;

    fn create_session_for_model<'a>(
        &'a self,
        cwd: &'a Path,
        _model: &'a RuntimeModelId,
    ) -> BackendFuture<'a, RuntimeSessionId> {
        self.create_session(cwd)
    }

    fn load_session<'a>(
''',
)
replace_once(
    router,
    '''    fn prompt<'a>(
''',
    '''    fn load_session_for_model<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        _model: &'a RuntimeModelId,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        self.load_session(session_id, cwd, sink)
    }

    fn prompt<'a>(
''',
)
replace_once(
    router,
    '''    fn load_session<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.load_session(session_id, cwd, sink).await?;
            Ok(())
        })
    }

    fn prompt<'a>(
''',
    '''    fn load_session<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.load_session(session_id, cwd, sink).await?;
            Ok(())
        })
    }

    fn create_session_for_model<'a>(
        &'a self,
        cwd: &'a Path,
        model: &'a RuntimeModelId,
    ) -> BackendFuture<'a, RuntimeSessionId> {
        Box::pin(async move {
            if model.provider != ProviderId::Cursor {
                return Err(RuntimeRouterError::ModelProviderMismatch {
                    backend: ProviderId::Cursor,
                    model_provider: model.provider,
                });
            }
            Ok(self.new_session_with_model(cwd, &model.model).await?)
        })
    }

    fn load_session_for_model<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        model: &'a RuntimeModelId,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            if model.provider != ProviderId::Cursor {
                return Err(RuntimeRouterError::ModelProviderMismatch {
                    backend: ProviderId::Cursor,
                    model_provider: model.provider,
                });
            }
            self.load_session_with_model(session_id, cwd, &model.model, sink)
                .await?;
            Ok(())
        })
    }

    fn prompt<'a>(
''',
)
replace_once(
    router,
    '''    #[error("runtime provider {0} is unavailable")]
    ProviderUnavailable(ProviderId),
''',
    '''    #[error("runtime provider {0} is unavailable")]
    ProviderUnavailable(ProviderId),
    #[error("backend {backend} cannot execute a model owned by {model_provider}")]
    ModelProviderMismatch {
        backend: ProviderId,
        model_provider: ProviderId,
    },
''',
)

lazy = "codex-rs/runtime-harness/src/lazy_cursor.rs"
replace_once(
    lazy,
    '''use crate::types::RuntimeInteractionHandler;
use crate::types::RuntimeSessionId;
''',
    '''use crate::types::RuntimeInteractionHandler;
use crate::types::RuntimeModelId;
use crate::types::RuntimeSessionId;
''',
)
replace_once(
    lazy,
    '''    fn load_session<'a>(
''',
    '''    fn create_session_for_model<'a>(
        &'a self,
        cwd: &'a Path,
        model: &'a RuntimeModelId,
    ) -> BackendFuture<'a, RuntimeSessionId> {
        Box::pin(async move {
            if model.provider != ProviderId::Cursor {
                return Err(RuntimeRouterError::ModelProviderMismatch {
                    backend: ProviderId::Cursor,
                    model_provider: model.provider,
                });
            }
            let backend = self.backend().await?;
            match backend.new_session_with_model(cwd, &model.model).await {
                Ok(session) => Ok(session),
                Err(error) if Self::is_transport_failure(&error) => {
                    let recovered = self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/new",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    Ok(recovered.new_session_with_model(cwd, &model.model).await?)
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    fn load_session<'a>(
''',
)
replace_once(
    lazy,
    '''    fn prompt<'a>(
''',
    '''    fn load_session_for_model<'a>(
        &'a self,
        session_id: &'a RuntimeSessionId,
        cwd: &'a Path,
        model: &'a RuntimeModelId,
        sink: Arc<dyn RuntimeEventSink>,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            if model.provider != ProviderId::Cursor {
                return Err(RuntimeRouterError::ModelProviderMismatch {
                    backend: ProviderId::Cursor,
                    model_provider: model.provider,
                });
            }
            let backend = self.backend().await?;
            match backend
                .load_session_with_model(session_id, cwd, &model.model, Arc::clone(&sink))
                .await
            {
                Ok(()) => Ok(()),
                Err(error) if Self::is_transport_failure(&error) => {
                    let recovered = self.recover_backend(&backend).await.map_err(|recovery| {
                        RuntimeRouterError::CursorRecoveryFailed {
                            operation: "session/load",
                            original: error.to_string(),
                            recovery: recovery.to_string(),
                        }
                    })?;
                    recovered
                        .load_session_with_model(session_id, cwd, &model.model, sink)
                        .await?;
                    Ok(())
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    fn prompt<'a>(
''',
)

test = "codex-rs/runtime-harness/tests/cursor_acp_mock.rs"
replace_once(
    test,
    '''    elif method == "session/new":
        send({"jsonrpc":"2.0","id":request_id,"result":{"sessionId":"mock-session"}})
    elif method == "session/load":
        send({"jsonrpc":"2.0","id":request_id,"result":{}})
''',
    '''    elif method == "session/new":
        send({"jsonrpc":"2.0","id":request_id,"result":{"sessionId":"mock-session","configOptions":[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"auto","options":[{"value":"auto","name":"Auto"},{"value":"composer","name":"Composer"}]}]}})
    elif method == "session/load":
        send({"jsonrpc":"2.0","id":request_id,"result":{"configOptions":[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"auto","options":[{"value":"auto","name":"Auto"},{"value":"composer","name":"Composer"}]}]}})
    elif method == "session/set_config_option":
        if params.get("configId") != "model" or params.get("value") != "composer":
            send({"jsonrpc":"2.0","id":request_id,"error":{"code":-32002,"message":"wrong model config"}})
        else:
            send({"jsonrpc":"2.0","id":request_id,"result":{}})
''',
)
replace_once(
    test,
    '''#[tokio::test]
async fn cursor_acp_cancel_bypasses_active_prompt_serialization() {
''',
    '''#[tokio::test]
async fn cursor_acp_applies_selected_model_through_session_config() {
    let temp = tempfile::tempdir().unwrap();
    let executable = write_mock_agent(temp.path());
    let backend = CursorAcpBackend::connect(CursorAcpConfig {
        executable: Some(executable),
        process_cwd: Some(temp.path().to_path_buf()),
    })
    .await
    .unwrap();

    let session = backend
        .new_session_with_model(temp.path(), "composer")
        .await
        .unwrap();
    assert_eq!(session.0, "mock-session");

    let collector = Arc::new(Collector::default());
    backend
        .load_session_with_model(&session, temp.path(), "composer", collector)
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test]
async fn cursor_acp_rejects_unavailable_selected_model() {
    let temp = tempfile::tempdir().unwrap();
    let executable = write_mock_agent(temp.path());
    let backend = CursorAcpBackend::connect(CursorAcpConfig {
        executable: Some(executable),
        process_cwd: Some(temp.path().to_path_buf()),
    })
    .await
    .unwrap();

    let error = backend
        .new_session_with_model(temp.path(), "not-a-real-model")
        .await
        .unwrap_err();
    assert!(matches!(error, CursorAcpError::ModelUnavailable(model) if model == "not-a-real-model"));
    backend.shutdown().await.unwrap();
}

#[tokio::test]
async fn cursor_acp_cancel_bypasses_active_prompt_serialization() {
''',
)

print("runtime harness model-config patch applied")
