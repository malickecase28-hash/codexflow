#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def replace_once(path: str, old: str, new: str) -> None:
    p = ROOT / path
    text = p.read_text()
    if new in text:
        return
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:100]!r}")
    p.write_text(text.replace(old, new, 1))


def write(path: str, content: str) -> None:
    p = ROOT / path
    p.parent.mkdir(parents=True, exist_ok=True)
    if p.exists() and p.read_text() == content:
        return
    p.write_text(content)


runtime_module = r'''//! TUI composition bridge for the provider-neutral multi-runtime harness.
//!
//! This module deliberately keeps provider selection process-wide. The Codex TUI
//! can own multiple `ChatWidget`s (side conversations, resumed sessions), but the
//! selected provider/account is a client-level concern because subswap activates
//! credentials in the native clients themselves.

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::model_catalog::ModelCatalog as TuiModelCatalog;
use async_trait::async_trait;
use codex_runtime_harness::AgentBackend;
use codex_runtime_harness::CursorAcpConfig;
use codex_runtime_harness::ModelDescriptor;
use codex_runtime_harness::PermissionOutcome;
use codex_runtime_harness::PermissionRequest;
use codex_runtime_harness::ProviderCapabilities;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::QuotaRefreshState;
use codex_runtime_harness::RuntimeEvent;
use codex_runtime_harness::RuntimeEventSink;
use codex_runtime_harness::RuntimeHarness;
use codex_runtime_harness::RuntimeInteractionHandler;
use codex_runtime_harness::RuntimeModelId;
use codex_runtime_harness::RuntimeRoute;
use codex_runtime_harness::RuntimeSelection;
use codex_runtime_harness::RuntimeSelectionStore;
use codex_runtime_harness::RuntimeSessionId;
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Semaphore;
use tokio::sync::oneshot;

struct UiRuntimeState {
    harness: Arc<RuntimeHarness>,
    selection: RwLock<RuntimeSelection>,
    external_session: AsyncMutex<Option<RuntimeSessionId>>,
    turn_gate: Arc<Semaphore>,
    default_openai_model: String,
}

static STATE: OnceLock<Arc<UiRuntimeState>> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct RuntimePermissionResponder {
    tx: Arc<Mutex<Option<oneshot::Sender<PermissionOutcome>>>>,
}

impl fmt::Debug for RuntimePermissionResponder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimePermissionResponder").finish_non_exhaustive()
    }
}

impl RuntimePermissionResponder {
    fn new(tx: oneshot::Sender<PermissionOutcome>) -> Self {
        Self {
            tx: Arc::new(Mutex::new(Some(tx))),
        }
    }

    pub(crate) fn resolve(&self, outcome: PermissionOutcome) {
        if let Ok(mut slot) = self.tx.lock()
            && let Some(tx) = slot.take()
        {
            let _ = tx.send(outcome);
        }
    }
}

pub(crate) fn initialize(default_model: &str, catalog: &TuiModelCatalog) -> Result<(), String> {
    if STATE.get().is_some() {
        return Ok(());
    }

    let default_model = RuntimeModelId::new(ProviderId::OpenAi, default_model)
        .map_err(|error| error.to_string())?;
    let codex_home = codex_utils_home_dir::find_codex_home().map_err(|error| error.to_string())?;
    let selection_path = codex_home.join("runtime-harness-selection.json");
    let store = RuntimeSelectionStore::new(selection_path.clone());
    let persisted = store.load().map_err(|error| error.to_string())?;
    let initial_selection = persisted.unwrap_or_else(|| RuntimeSelection::new(default_model.clone()));
    let harness = Arc::new(
        RuntimeHarness::embedded(default_model.clone(), selection_path, CursorAcpConfig::default())
            .map_err(|error| error.to_string())?,
    );

    if let Ok(presets) = catalog.try_list_models() {
        let models = presets
            .into_iter()
            .filter_map(|preset| {
                let id = RuntimeModelId::new(ProviderId::OpenAi, preset.model).ok()?;
                Some(ModelDescriptor {
                    id,
                    display_name: preset.display_name,
                    capabilities: ProviderCapabilities::for_provider(ProviderId::OpenAi),
                    parameters: Vec::new(),
                    metadata: serde_json::json!({"description": preset.description}),
                })
            })
            .collect::<Vec<_>>();
        let harness_for_catalog = Arc::clone(&harness);
        tokio::spawn(async move {
            if let Err(error) = harness_for_catalog.replace_openai_models(models).await {
                tracing::debug!(%error, "failed to seed runtime harness OpenAI catalog");
            }
        });
    }

    let state = Arc::new(UiRuntimeState {
        harness,
        selection: RwLock::new(initial_selection),
        external_session: AsyncMutex::new(None),
        turn_gate: Arc::new(Semaphore::new(1)),
        default_openai_model: default_model.model,
    });
    let _ = STATE.set(state);
    Ok(())
}

fn state() -> Result<Arc<UiRuntimeState>, String> {
    STATE
        .get()
        .cloned()
        .ok_or_else(|| "multi-runtime harness is not initialized".to_string())
}

pub(crate) fn active_provider() -> ProviderId {
    let Some(state) = STATE.get() else {
        return ProviderId::OpenAi;
    };
    state
        .selection
        .read()
        .map(|selection| selection.provider())
        .unwrap_or(ProviderId::OpenAi)
}

pub(crate) fn current_selection() -> Option<RuntimeSelection> {
    STATE
        .get()
        .and_then(|state| state.selection.read().ok().map(|selection| selection.clone()))
}

fn replace_selection(state: &UiRuntimeState, selection: RuntimeSelection) {
    if let Ok(mut guard) = state.selection.write() {
        *guard = selection;
    }
}

async fn clear_external_session(state: &UiRuntimeState) {
    *state.external_session.lock().await = None;
}

fn info(tx: &AppEventSender, message: impl Into<String>) {
    tx.send(AppEvent::RuntimeHarnessNotice {
        message: message.into(),
        is_error: false,
    });
}

fn error(tx: &AppEventSender, message: impl Into<String>) {
    tx.send(AppEvent::RuntimeHarnessNotice {
        message: message.into(),
        is_error: true,
    });
}

pub(crate) fn show_provider(tx: AppEventSender) {
    match current_selection() {
        Some(selection) => info(
            &tx,
            format!(
                "Runtime provider: {}\nModel: {}\nAccount: {}",
                selection.provider(),
                selection.model,
                selection.account_id.as_deref().unwrap_or("default/native")
            ),
        ),
        None => error(&tx, "multi-runtime harness is not initialized"),
    }
}

pub(crate) fn select_provider(tx: AppEventSender, provider: &str) {
    let provider = provider.trim().to_string();
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => return error(&tx, message),
        };
        let provider = match provider.parse::<ProviderId>() {
            Ok(provider) => provider,
            Err(parse_error) => return error(&tx, parse_error.to_string()),
        };
        let current = match state.selection.read() {
            Ok(selection) => selection.clone(),
            Err(_) => return error(&tx, "runtime selection lock is poisoned"),
        };
        if current.provider() == provider {
            return info(&tx, format!("Runtime provider already set to {provider}."));
        }
        let model_name = match provider {
            ProviderId::OpenAi => state.default_openai_model.clone(),
            ProviderId::Cursor => "auto".to_string(),
        };
        let model = match RuntimeModelId::new(provider, model_name) {
            Ok(model) => model,
            Err(model_error) => return error(&tx, model_error.to_string()),
        };
        match state.harness.select_provider(model).await {
            Ok(selection) => {
                replace_selection(&state, selection.clone());
                clear_external_session(&state).await;
                tx.send(AppEvent::RuntimeHarnessSelectionChanged {
                    selection,
                    message: "Runtime provider changed explicitly.".to_string(),
                });
            }
            Err(select_error) => error(&tx, select_error.to_string()),
        }
    });
}

pub(crate) fn show_models(tx: AppEventSender) {
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => return error(&tx, message),
        };
        let cursor_error = state.harness.refresh_cursor_models().await.err();
        let catalog = state.harness.catalog().await;
        let mut lines = vec!["Available runtime models:".to_string(), "OpenAI / native:".to_string()];
        for model in catalog.models_for_provider(ProviderId::OpenAi) {
            lines.push(format!("  {} — {}", model.id, model.display_name));
        }
        lines.push("Cursor / ACP:".to_string());
        for model in catalog.models_for_provider(ProviderId::Cursor) {
            lines.push(format!("  {} — {}", model.id, model.display_name));
        }
        if let Some(cursor_error) = cursor_error {
            lines.push(format!("  Cursor discovery unavailable: {cursor_error}"));
            lines.push("  cursor/auto remains selectable explicitly.".to_string());
        }
        lines.push("Use /model <provider/model> to select.".to_string());
        info(&tx, lines.join("\n"));
    });
}

pub(crate) fn select_model(tx: AppEventSender, qualified: &str) {
    let qualified = qualified.trim().to_string();
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => return error(&tx, message),
        };
        let model = match qualified.parse::<RuntimeModelId>() {
            Ok(model) => model,
            Err(parse_error) => return error(&tx, parse_error.to_string()),
        };
        let current = match state.selection.read() {
            Ok(selection) => selection.clone(),
            Err(_) => return error(&tx, "runtime selection lock is poisoned"),
        };
        let result = if current.provider() == model.provider {
            state.harness.select_model(model).await
        } else {
            state.harness.select_provider(model).await
        };
        match result {
            Ok(selection) => {
                replace_selection(&state, selection.clone());
                clear_external_session(&state).await;
                tx.send(AppEvent::RuntimeHarnessSelectionChanged {
                    selection,
                    message: "Runtime model changed.".to_string(),
                });
            }
            Err(select_error) => error(&tx, select_error.to_string()),
        }
    });
}

pub(crate) fn account_command(tx: AppEventSender, args: &str) {
    let args = args.trim().to_string();
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => return error(&tx, message),
        };
        let selection = match state.selection.read() {
            Ok(selection) => selection.clone(),
            Err(_) => return error(&tx, "runtime selection lock is poisoned"),
        };
        let mut parts = args.split_whitespace();
        match parts.next() {
            None | Some("list") | Some("status") => {
                let mut lines = vec![format!("Active runtime: {}", selection.provider())];
                for provider in [ProviderId::OpenAi, ProviderId::Cursor] {
                    match state.harness.broker().list_accounts(provider).await {
                        Ok(accounts) => {
                            lines.push(format!("{provider} accounts:"));
                            if accounts.is_empty() {
                                lines.push("  (none imported)".to_string());
                            }
                            for account in accounts {
                                let marker = if account.active { "*" } else { " " };
                                lines.push(format!(" {marker} {provider}/{} — {}", account.id, account.label));
                            }
                        }
                        Err(account_error) => lines.push(format!("{provider}: {account_error}")),
                    }
                }
                lines.push("Use /account use <provider>/<id> or /account add cursor.".to_string());
                info(&tx, lines.join("\n"));
            }
            Some("use") => {
                let Some(target) = parts.next() else {
                    return error(&tx, "Usage: /account use <provider>/<id>");
                };
                let Some((provider, account_id)) = target.split_once('/') else {
                    return error(&tx, "Account id must be qualified as provider/id");
                };
                let provider = match provider.parse::<ProviderId>() {
                    Ok(provider) => provider,
                    Err(parse_error) => return error(&tx, parse_error.to_string()),
                };
                if provider != selection.provider() {
                    return error(
                        &tx,
                        format!(
                            "Account switch cannot change runtime provider (active {}). Use /provider {} first.",
                            selection.provider(),
                            provider
                        ),
                    );
                }
                match state.harness.activate_account(provider, account_id.to_string()).await {
                    Ok(activation) => {
                        let next = state.harness.selection().await;
                        replace_selection(&state, next.clone());
                        clear_external_session(&state).await;
                        tx.send(AppEvent::RuntimeHarnessSelectionChanged {
                            selection: next,
                            message: format!(
                                "Activated {provider}/{} (generation {}).",
                                activation.account_id, activation.generation
                            ),
                        });
                    }
                    Err(activation_error) => error(&tx, activation_error.to_string()),
                }
            }
            Some("add") => match parts.next() {
                Some("cursor") => match state.harness.login_cursor(None).await {
                    Ok(imported) => {
                        let next = state.harness.selection().await;
                        replace_selection(&state, next.clone());
                        clear_external_session(&state).await;
                        tx.send(AppEvent::RuntimeHarnessSelectionChanged {
                            selection: next,
                            message: format!("Imported Cursor account {}.", imported.account.label),
                        });
                    }
                    Err(login_error) => error(&tx, login_error.to_string()),
                },
                Some("openai") | Some("codex") => error(
                    &tx,
                    "Use the existing Codex login flow, then run /account import-openai to import it into the embedded broker.",
                ),
                _ => error(&tx, "Usage: /account add cursor"),
            },
            Some("import-openai") => match state
                .harness
                .import_after_native_login(ProviderId::OpenAi, None)
                .await
            {
                Ok(imported) => info(&tx, format!("Imported OpenAI account {}.", imported.account.label)),
                Err(import_error) => error(&tx, import_error.to_string()),
            },
            Some(other) => error(
                &tx,
                format!("Unknown account command {other:?}. Use list, status, use, add, or import-openai."),
            ),
        }
    });
}

pub(crate) fn quota_command(tx: AppEventSender, provider: Option<&str>) {
    let provider = provider.map(str::to_string);
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => return error(&tx, message),
        };
        let provider = match provider {
            Some(raw) if !raw.trim().is_empty() => match raw.parse::<ProviderId>() {
                Ok(provider) => provider,
                Err(parse_error) => return error(&tx, parse_error.to_string()),
            },
            _ => match state.selection.read() {
                Ok(selection) => selection.provider(),
                Err(_) => return error(&tx, "runtime selection lock is poisoned"),
            },
        };
        match state.harness.quota_snapshot(provider).await {
            Ok(snapshot) => {
                let mut lines = vec![format!("Quota — {provider}")];
                if snapshot.accounts.is_empty() {
                    lines.push("  (no imported accounts)".to_string());
                }
                for account in snapshot.accounts {
                    lines.push(format!(
                        "{}{} — {:?}",
                        if account.account.active { "* " } else { "  " },
                        account.account.label,
                        account.state
                    ));
                    if account.quotas.is_empty() {
                        lines.push("    quota unavailable".to_string());
                    }
                    for quota in account.quotas {
                        let percentage = quota
                            .usage_ratio()
                            .map(|ratio| format!("{:.1}%", ratio * 100.0))
                            .unwrap_or_else(|| "unknown".to_string());
                        lines.push(format!(
                            "    {:?}: {percentage} used, status {:?}",
                            quota.window, quota.status
                        ));
                    }
                }
                info(&tx, lines.join("\n"));
            }
            Err(quota_error) => error(&tx, quota_error.to_string()),
        }
    });
}

pub(crate) fn autoswap_command(tx: AppEventSender, args: &str) {
    let args = args.trim().to_ascii_lowercase();
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => return error(&tx, message),
        };
        if !args.is_empty() && args != "now" && args != "status" {
            return error(&tx, "Usage: /autoswap [status|now]");
        }
        if args != "now" {
            return info(
                &tx,
                "Autoswap uses embedded subswap policy and is same-provider only. Run /autoswap now to evaluate it immediately.",
            );
        }
        match state.harness.auto_swap_default().await {
            Ok(summary) => {
                let next = state.harness.selection().await;
                replace_selection(&state, next.clone());
                clear_external_session(&state).await;
                tx.send(AppEvent::RuntimeHarnessSelectionChanged {
                    selection: next,
                    message: format!("Autoswap decision: {summary}"),
                });
            }
            Err(auto_error) => error(&tx, auto_error.to_string()),
        }
    });
}

struct TuiRuntimeSink {
    tx: AppEventSender,
}

impl RuntimeEventSink for TuiRuntimeSink {
    fn emit(&self, event: RuntimeEvent) {
        self.tx.send(AppEvent::RuntimeHarnessEvent(event));
    }
}

struct TuiRuntimeInteractions {
    tx: AppEventSender,
}

#[async_trait]
impl RuntimeInteractionHandler for TuiRuntimeInteractions {
    async fn decide_permission(&self, request: &PermissionRequest) -> PermissionOutcome {
        let (tx, rx) = oneshot::channel();
        self.tx.send(AppEvent::RuntimeHarnessPermission {
            request: request.clone(),
            responder: RuntimePermissionResponder::new(tx),
        });
        rx.await.unwrap_or(PermissionOutcome::RejectOnce)
    }

    async fn handle_cursor_extension(&self, method: &str, params: &Value) -> Option<Value> {
        self.tx.send(AppEvent::RuntimeHarnessEvent(RuntimeEvent::CursorExtension {
            method: method.to_string(),
            params: params.clone(),
        }));
        None
    }
}

pub(crate) fn submit_cursor_turn(tx: AppEventSender, cwd: PathBuf, prompt: String) {
    tokio::spawn(async move {
        let state = match state() {
            Ok(state) => state,
            Err(message) => {
                error(&tx, message);
                return tx.send(AppEvent::RuntimeHarnessTurnFinished {
                    result: Err("runtime harness unavailable".to_string()),
                });
            }
        };
        let _permit = match Arc::clone(&state.turn_gate).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                return tx.send(AppEvent::RuntimeHarnessTurnFinished {
                    result: Err("runtime turn coordinator unavailable".to_string()),
                });
            }
        };
        let route = match state.harness.route_current().await {
            Ok(RuntimeRoute::External(backend)) => backend,
            Ok(RuntimeRoute::NativeOpenAi) => {
                return tx.send(AppEvent::RuntimeHarnessTurnFinished {
                    result: Err("external runtime turn was routed to native OpenAI".to_string()),
                });
            }
            Err(route_error) => {
                return tx.send(AppEvent::RuntimeHarnessTurnFinished {
                    result: Err(route_error.to_string()),
                });
            }
        };

        let session_id = {
            let existing = state.external_session.lock().await.clone();
            match existing {
                Some(session_id) => session_id,
                None => match backend.create_session(&cwd).await {
                    Ok(session_id) => {
                        *state.external_session.lock().await = Some(session_id.clone());
                        session_id
                    }
                    Err(session_error) => {
                        return tx.send(AppEvent::RuntimeHarnessTurnFinished {
                            result: Err(session_error.to_string()),
                        });
                    }
                },
            }
        };

        let supervised = state
            .harness
            .supervisor()
            .begin(ProviderId::Cursor, session_id.clone());
        let sink: Arc<dyn RuntimeEventSink> = Arc::new(TuiRuntimeSink { tx: tx.clone() });
        let sink = state.harness.supervisor().guarded_sink(&supervised, sink);
        let interactions: Arc<dyn RuntimeInteractionHandler> =
            Arc::new(TuiRuntimeInteractions { tx: tx.clone() });
        let result = backend
            .prompt(&session_id, &prompt, sink, interactions)
            .await
            .map(|_| ())
            .map_err(|prompt_error| prompt_error.to_string());
        if result.is_err() {
            *state.external_session.lock().await = None;
            let _ = backend.shutdown().await;
            state.harness.supervisor().invalidate();
        }
        tx.send(AppEvent::RuntimeHarnessTurnFinished { result });
    });
}

pub(crate) fn cancel_cursor_turn(tx: AppEventSender) -> bool {
    let Some(state) = STATE.get().cloned() else {
        return false;
    };
    if active_provider() != ProviderId::Cursor {
        return false;
    }
    tokio::spawn(async move {
        let session_id = state.external_session.lock().await.clone();
        let Some(session_id) = session_id else {
            return;
        };
        match state.harness.route_current().await {
            Ok(RuntimeRoute::External(backend)) => match backend.cancel(&session_id).await {
                Ok(()) => info(&tx, "Cursor turn cancelled."),
                Err(cancel_error) => error(&tx, cancel_error.to_string()),
            },
            Ok(RuntimeRoute::NativeOpenAi) => error(&tx, "Cursor cancel routed to native OpenAI"),
            Err(route_error) => error(&tx, route_error.to_string()),
        }
    });
    true
}
'''
write("codex-rs/tui/src/runtime_harness.rs", runtime_module)

chatwidget_runtime = r'''use super::*;
use codex_runtime_harness::PermissionOutcome;
use codex_runtime_harness::PermissionRequest;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::RuntimeEvent;
use codex_runtime_harness::RuntimeSelection;

impl ChatWidget {
    pub(super) fn handle_runtime_harness_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::AgentMessageChunk { text } => self.on_agent_message_delta(text),
            RuntimeEvent::ToolCall { raw } => self.add_info_message(
                format!("Cursor tool call: {raw}"),
                Some("Permission prompts remain controlled by CodexFlow.".to_string()),
            ),
            RuntimeEvent::ToolCallUpdate { raw } => {
                tracing::debug!(%raw, "Cursor tool call update");
            }
            RuntimeEvent::Plan { raw } => self.add_info_message(format!("Cursor plan: {raw}"), None),
            RuntimeEvent::UsageUpdate { raw } => {
                tracing::debug!(%raw, "Cursor usage update");
            }
            RuntimeEvent::PermissionRequest { .. } => {
                // The ACP transport routes permission requests through the interaction
                // handler; this event is retained for observability only.
            }
            RuntimeEvent::CursorExtension { method, params } => {
                tracing::debug!(%method, %params, "Cursor ACP extension event");
            }
            RuntimeEvent::SessionUpdate { update_type, raw } => {
                tracing::debug!(%update_type, %raw, "Cursor session update");
            }
            RuntimeEvent::Completed { stop_reason } => {
                tracing::debug!(?stop_reason, "Cursor ACP prompt completed");
            }
            RuntimeEvent::ProviderError { message } => self.add_error_message(message),
        }
        self.request_redraw();
    }

    pub(super) fn handle_runtime_harness_turn_finished(&mut self, result: Result<(), String>) {
        if let Err(message) = result {
            self.add_error_message(format!("Cursor runtime failed: {message}"));
        }
        self.on_task_complete(
            /*last_agent_message*/ None,
            /*duration_ms*/ None,
            /*from_replay*/ false,
        );
    }

    pub(super) fn handle_runtime_harness_selection_changed(
        &mut self,
        selection: RuntimeSelection,
        message: String,
    ) {
        let account = selection.account_id.as_deref().unwrap_or("default/native");
        self.add_info_message(
            format!(
                "{message}\nProvider: {}\nModel: {}\nAccount: {account}",
                selection.provider(), selection.model
            ),
            None,
        );
        self.request_redraw();
    }

    pub(super) fn handle_runtime_harness_permission(
        &mut self,
        request: PermissionRequest,
        responder: crate::runtime_harness::RuntimePermissionResponder,
    ) {
        let tool_summary = request
            .tool_call
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Cursor tool request".to_string());
        let mut items = Vec::new();
        for option in request.options {
            let normalized = match option.option_id.as_str() {
                "allow-once" => Some(PermissionOutcome::AllowOnce),
                "allow-always" => Some(PermissionOutcome::AllowAlways),
                "reject-once" | "reject" => Some(PermissionOutcome::RejectOnce),
                _ => None,
            };
            let Some(outcome) = normalized else {
                continue;
            };
            let responder = responder.clone();
            items.push(SelectionItem {
                name: option.name.unwrap_or_else(|| option.option_id.clone()),
                description: option.kind,
                actions: vec![Box::new(move |_| responder.resolve(outcome))],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
        if items.is_empty() {
            responder.resolve(PermissionOutcome::RejectOnce);
            self.add_error_message("Cursor requested an unsupported permission option set.".to_string());
            return;
        }
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Cursor requests permission".to_string()),
            subtitle: Some(tool_summary),
            items,
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(super) fn dispatch_runtime_command(&mut self, cmd: SlashCommand, args: Option<&str>) -> bool {
        let tx = self.app_event_tx.clone();
        match cmd {
            SlashCommand::Provider => match args.map(str::trim).filter(|arg| !arg.is_empty()) {
                Some(provider) => crate::runtime_harness::select_provider(tx, provider),
                None => crate::runtime_harness::show_provider(tx),
            },
            SlashCommand::Account => {
                crate::runtime_harness::account_command(tx, args.unwrap_or_default())
            }
            SlashCommand::Quota => crate::runtime_harness::quota_command(
                tx,
                args.map(str::trim).filter(|arg| !arg.is_empty()),
            ),
            SlashCommand::Autoswap => {
                crate::runtime_harness::autoswap_command(tx, args.unwrap_or_default())
            }
            SlashCommand::Model => match args.map(str::trim).filter(|arg| !arg.is_empty()) {
                Some(model) if model.contains('/') => crate::runtime_harness::select_model(tx, model),
                Some(_) => return false,
                None => crate::runtime_harness::show_models(tx),
            },
            _ => return false,
        }
        true
    }

    pub(super) fn runtime_provider_is_cursor(&self) -> bool {
        crate::runtime_harness::active_provider() == ProviderId::Cursor
    }
}
'''
write("codex-rs/tui/src/chatwidget/runtime_harness.rs", chatwidget_runtime)

replace_once(
    "codex-rs/tui/Cargo.toml",
    "codex-core = { workspace = true }\n",
    "codex-core = { workspace = true }\ncodex-runtime-harness = { path = \"../runtime-harness\" }\nasync-trait = \"0.1\"\n",
)

replace_once(
    "codex-rs/tui/src/lib.rs",
    "mod render;\n",
    "mod render;\nmod runtime_harness;\n",
)

replace_once(
    "codex-rs/tui/src/chatwidget.rs",
    "mod protocol;\n",
    "mod protocol;\nmod runtime_harness;\n",
)

replace_once(
    "codex-rs/tui/src/chatwidget/constructor.rs",
    "        let model_for_header = model.clone();\n",
    "        let model_for_header = model.clone();\n        #[cfg(not(test))]\n        if let Err(error) = crate::runtime_harness::initialize(&model_for_header, model_catalog.as_ref()) {\n            tracing::warn!(%error, \"failed to initialize multi-runtime harness; native OpenAI remains available\");\n        }\n",
)

# Slash command surface.
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "    Model,\n    Ide,\n",
    "    Model,\n    Provider,\n    Account,\n    Quota,\n    Autoswap,\n    Ide,\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "            SlashCommand::Model => \"choose what model and reasoning effort to use\",\n",
    "            SlashCommand::Model => \"choose a provider-qualified runtime model\",\n            SlashCommand::Provider => \"show or explicitly switch runtime provider\",\n            SlashCommand::Account => \"list, import, or activate provider accounts\",\n            SlashCommand::Quota => \"show embedded provider account quota\",\n            SlashCommand::Autoswap => \"inspect or run same-provider quota autoswap\",\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "                | SlashCommand::Review\n",
    "                | SlashCommand::Review\n                | SlashCommand::Model\n                | SlashCommand::Provider\n                | SlashCommand::Account\n                | SlashCommand::Quota\n                | SlashCommand::Autoswap\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "            SlashCommand::Diff\n            | SlashCommand::Resume\n            | SlashCommand::Model\n",
    "            SlashCommand::Diff\n            | SlashCommand::Resume\n            | SlashCommand::Quota\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "            SlashCommand::New\n            | SlashCommand::Archive\n",
    "            SlashCommand::New\n            | SlashCommand::Provider\n            | SlashCommand::Account\n            | SlashCommand::Autoswap\n            | SlashCommand::Model\n            | SlashCommand::Archive\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "                | SlashCommand::Usage\n                | SlashCommand::Ide\n",
    "                | SlashCommand::Usage\n                | SlashCommand::Quota\n                | SlashCommand::Ide\n",
)

# Bare and inline slash dispatch.
replace_once(
    "codex-rs/tui/src/chatwidget/slash_dispatch.rs",
    "            SlashCommand::Model => {\n                self.open_model_popup();\n                self.defer_input_until_settings_applied();\n            }\n",
    "            SlashCommand::Model => {\n                self.dispatch_runtime_command(SlashCommand::Model, None);\n            }\n            SlashCommand::Provider | SlashCommand::Account | SlashCommand::Quota | SlashCommand::Autoswap => {\n                self.dispatch_runtime_command(cmd, None);\n            }\n",
)
replace_once(
    "codex-rs/tui/src/chatwidget/slash_dispatch.rs",
    "        match cmd {\n            SlashCommand::Export if trimmed.is_empty() => self.show_transcript_export_popup(),\n",
    "        match cmd {\n            SlashCommand::Model | SlashCommand::Provider | SlashCommand::Account | SlashCommand::Quota | SlashCommand::Autoswap => {\n                if !self.dispatch_runtime_command(cmd, Some(trimmed)) {\n                    self.add_error_message(format!(\"Usage: /{} <provider/model>\", cmd.command()));\n                }\n            }\n            SlashCommand::Export if trimmed.is_empty() => self.show_transcript_export_popup(),\n",
)
replace_once(
    "codex-rs/tui/src/chatwidget/slash_dispatch.rs",
    "            | SlashCommand::Model\n            | SlashCommand::Personality\n",
    "            | SlashCommand::Model\n            | SlashCommand::Provider\n            | SlashCommand::Account\n            | SlashCommand::Quota\n            | SlashCommand::Autoswap\n            | SlashCommand::Personality\n",
)

# App event bridge.
replace_once(
    "codex-rs/tui/src/app_event.rs",
    "use crate::history_cell::HistoryCell;\n",
    "use crate::history_cell::HistoryCell;\nuse codex_runtime_harness::PermissionRequest;\nuse codex_runtime_harness::RuntimeEvent;\nuse codex_runtime_harness::RuntimeSelection;\n",
)
replace_once(
    "codex-rs/tui/src/app_event.rs",
    "    InsertHistoryCell(Box<dyn HistoryCell>),\n",
    "    InsertHistoryCell(Box<dyn HistoryCell>),\n\n    RuntimeHarnessEvent(RuntimeEvent),\n    RuntimeHarnessTurnFinished {\n        result: Result<(), String>,\n    },\n    RuntimeHarnessPermission {\n        request: PermissionRequest,\n        responder: crate::runtime_harness::RuntimePermissionResponder,\n    },\n    RuntimeHarnessSelectionChanged {\n        selection: RuntimeSelection,\n        message: String,\n    },\n    RuntimeHarnessNotice {\n        message: String,\n        is_error: bool,\n    },\n",
)
replace_once(
    "codex-rs/tui/src/app/event_dispatch.rs",
    "            AppEvent::InsertHistoryCell(cell) => {\n                self.insert_history_cell(tui, cell);\n            }\n",
    "            AppEvent::InsertHistoryCell(cell) => {\n                self.insert_history_cell(tui, cell);\n            }\n            AppEvent::RuntimeHarnessEvent(event) => {\n                self.chat_widget.handle_runtime_harness_event(event);\n            }\n            AppEvent::RuntimeHarnessTurnFinished { result } => {\n                self.chat_widget.handle_runtime_harness_turn_finished(result);\n            }\n            AppEvent::RuntimeHarnessPermission { request, responder } => {\n                self.chat_widget.handle_runtime_harness_permission(request, responder);\n            }\n            AppEvent::RuntimeHarnessSelectionChanged { selection, message } => {\n                self.chat_widget.handle_runtime_harness_selection_changed(selection, message);\n            }\n            AppEvent::RuntimeHarnessNotice { message, is_error } => {\n                if is_error {\n                    self.chat_widget.add_error_message(message);\n                } else {\n                    self.chat_widget.add_info_message(message, None);\n                }\n            }\n",
)

# External prompt intercept: Cursor gets the same submitted-input UX, but no native
# AppCommand::user_turn is emitted.
anchor = """        // Special-case: \"!cmd\" executes a local shell command instead of sending to the model.\n        if shell_escape_policy == ShellEscapePolicy::Allow\n            && let Some(stripped) = text.strip_prefix('!')\n        {\n            let app_command = match self.submit_shell_command_with_history(stripped, &text) {\n                QueueDrain::Continue => None,\n                QueueDrain::Stop => Some(AppCommand::run_user_shell_command(\n                    stripped.trim().to_string(),\n                )),\n            };\n            return (app_command.is_some(), app_command);\n        }\n\n"""
insert = anchor + """        if self.runtime_provider_is_cursor() {\n            if !local_images.is_empty() || !remote_image_urls.is_empty() {\n                self.add_error_message(\n                    \"Cursor ACP image attachments are not enabled by this backend capability. Remove the images or switch to an OpenAI model.\".to_string(),\n                );\n                self.restore_user_message_to_composer(user_message_for_restore(\n                    UserMessage {\n                        text,\n                        local_images,\n                        remote_image_urls,\n                        text_elements,\n                        mention_bindings,\n                    },\n                    &history_record,\n                ));\n                return (false, None);\n            }\n            if text.trim().is_empty() {\n                return (false, None);\n            }\n            let submitted_message = UserMessage {\n                text: text.clone(),\n                local_images,\n                remote_image_urls,\n                text_elements,\n                mention_bindings,\n            };\n            if render_in_history {\n                self.clear_recap_loading();\n                self.on_user_message_display(user_message_display_for_history(\n                    submitted_message.clone(),\n                    &history_record,\n                ));\n            }\n            let encoded_mentions = submitted_message\n                .mention_bindings\n                .iter()\n                .map(|binding| LinkedMention {\n                    sigil: binding.sigil,\n                    mention: binding.mention.clone(),\n                    path: binding.path.clone(),\n                })\n                .collect::<Vec<_>>();\n            let history = match &history_record {\n                UserMessageHistoryRecord::UserMessageText if !submitted_message.text.is_empty() => {\n                    Some((&submitted_message.text, submitted_message.text_elements.as_slice()))\n                }\n                UserMessageHistoryRecord::Override(history) if !history.text.is_empty() => {\n                    Some((&history.text, history.text_elements.as_slice()))\n                }\n                UserMessageHistoryRecord::UserMessageText | UserMessageHistoryRecord::Override(_) => None,\n            };\n            if let Some((history_text, elements)) = history {\n                self.append_message_history_entry(encode_history_mentions_at_elements(\n                    history_text,\n                    &encoded_mentions,\n                    elements,\n                ));\n            }\n            self.on_task_started();\n            crate::runtime_harness::submit_cursor_turn(\n                self.app_event_tx.clone(),\n                self.config.cwd.to_path_buf(),\n                submitted_message.text,\n            );\n            self.transcript.needs_final_message_separator = false;\n            return (true, None);\n        }\n\n"""
replace_once("codex-rs/tui/src/chatwidget/input_submission.rs", anchor, insert)

# Cursor Ctrl+C cancellation before native interrupt submission.
replace_once(
    "codex-rs/tui/src/chatwidget/interaction.rs",
    "            self.input_queue.submit_pending_steers_after_interrupt = true;\n            if self.submit_op(AppCommand::interrupt()) {\n",
    "            self.input_queue.submit_pending_steers_after_interrupt = true;\n            if crate::runtime_harness::cancel_cursor_turn(self.app_event_tx.clone()) {\n                self.pause_active_goal_for_interrupt();\n                return;\n            }\n            if self.submit_op(AppCommand::interrupt()) {\n",
)

print("runtime harness TUI integration patch applied")
