use super::*;
use codex_app_server_protocol::AccountLoginCompletedNotification;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::RuntimeAutoSwapDecision;
use codex_runtime_harness::RuntimeModelId;
use std::str::FromStr;

const PROVIDER_USAGE: &str = "Usage: /provider [openai|cursor]";
const MODEL_USAGE: &str = "Usage: /model [model|provider/model]";
const ACCOUNT_USAGE: &str =
    "Usage: /account [list|status|add [provider]|use <provider/id>|remove <provider/id>]";

impl App {
    pub(super) async fn handle_runtime_slash_command(
        &mut self,
        command: String,
        args: String,
    ) {
        let result = match command.as_str() {
            "provider" => self.runtime_provider_command(&args).await,
            "model" => self.runtime_model_command(&args).await,
            "account" => self.runtime_account_command(&args).await,
            "quota" => self.runtime_quota_command(&args).await,
            "autoswap" => self.runtime_autoswap_command(&args).await,
            _ => Err(color_eyre::eyre::eyre!(
                "unknown runtime command /{command}"
            )),
        };
        if let Err(error) = result {
            self.chat_widget.add_error_message(error.to_string());
        }
    }

    pub(super) async fn handle_runtime_openai_login_completed(
        &mut self,
        notification: &AccountLoginCompletedNotification,
    ) -> bool {
        let Some(login_id) = notification.login_id.as_deref() else {
            return false;
        };
        if self.pending_runtime_openai_login.as_deref() != Some(login_id) {
            return false;
        }
        self.pending_runtime_openai_login = None;

        if !notification.success {
            self.chat_widget.add_error_message(
                notification
                    .error
                    .clone()
                    .unwrap_or_else(|| "ChatGPT login failed.".to_string()),
            );
            return true;
        }

        match self.runtime_bridge.import_completed_openai_login().await {
            Ok(selection) => {
                self.chat_widget.add_info_message(
                    format!(
                        "OpenAI account added through Codex login. Active runtime: {}",
                        selection.model.qualified()
                    ),
                    Some("Use /account list openai to inspect saved accounts.".to_string()),
                );
            }
            Err(error) => {
                self.chat_widget.add_error_message(format!(
                    "ChatGPT login succeeded, but importing the account into the runtime harness failed: {error}"
                ));
            }
        }
        true
    }

    async fn runtime_provider_command(&mut self, args: &str) -> color_eyre::Result<()> {
        let trimmed = args.trim();
        let current = self.runtime_bridge.selection().await;
        if trimmed.is_empty() {
            self.chat_widget.add_info_message(
                format!(
                    "Runtime provider: {} ({})",
                    current.provider(),
                    current.model.qualified()
                ),
                Some(PROVIDER_USAGE.to_string()),
            );
            return Ok(());
        }

        let provider = ProviderId::from_str(trimmed)?;
        if provider == current.provider() {
            self.chat_widget.add_info_message(
                format!("Runtime provider is already {provider}."),
                Some(MODEL_USAGE.to_string()),
            );
            return Ok(());
        }

        let target = match provider {
            ProviderId::OpenAi => RuntimeModelId::new(
                ProviderId::OpenAi,
                self.chat_widget.current_model().to_string(),
            )?,
            ProviderId::Cursor => {
                let models = self.runtime_bridge.refresh_cursor_models().await?;
                let Some(model) = models.into_iter().next() else {
                    return Err(color_eyre::eyre::eyre!(
                        "Cursor returned no models for the active account"
                    ));
                };
                model.id
            }
        };

        let selection = self.runtime_bridge.select_provider(target).await?;
        self.chat_widget.add_info_message(
            format!(
                "Runtime provider changed to {} ({})",
                selection.provider(),
                selection.model.qualified()
            ),
            Some("Use /model to inspect or change the model.".to_string()),
        );
        Ok(())
    }

    async fn runtime_model_command(&mut self, args: &str) -> color_eyre::Result<()> {
        let trimmed = args.trim();
        let selection = self.runtime_bridge.selection().await;
        if trimmed.is_empty() {
            match selection.provider() {
                ProviderId::OpenAi => {
                    self.chat_widget.open_model_popup();
                }
                ProviderId::Cursor => {
                    let models = self.runtime_bridge.refresh_cursor_models().await?;
                    if models.is_empty() {
                        self.chat_widget.add_error_message(
                            "Cursor returned no models for the active account".to_string(),
                        );
                    } else {
                        let rows = models
                            .into_iter()
                            .map(|model| {
                                let selected = if model.id == selection.model { " *" } else { "" };
                                format!("{} - {}{}", model.id.qualified(), model.display_name, selected)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.chat_widget.add_info_message(
                            format!("Cursor models:\n{rows}"),
                            Some("Select with /model cursor/<model>.".to_string()),
                        );
                    }
                }
            }
            return Ok(());
        }

        let model = if trimmed.contains('/') {
            RuntimeModelId::from_str(trimmed)?
        } else {
            RuntimeModelId::new(selection.provider(), trimmed)?
        };
        if model.provider != selection.provider() {
            return Err(color_eyre::eyre::eyre!(
                "model {} belongs to {}; switch providers with /provider first",
                model.qualified(),
                model.provider
            ));
        }

        let next = self.runtime_bridge.select_model(model.clone()).await?;
        if model.provider == ProviderId::OpenAi {
            self.app_event_tx.send(AppEvent::UpdateModel(model.model));
        } else {
            self.chat_widget.add_info_message(
                format!("Cursor model changed to {}", next.model.qualified()),
                /*hint*/ None,
            );
        }
        Ok(())
    }

    async fn runtime_account_command(&mut self, args: &str) -> color_eyre::Result<()> {
        let trimmed = args.trim();
        let mut parts = trimmed.split_whitespace();
        let action = parts.next().unwrap_or("status").to_ascii_lowercase();
        let selection = self.runtime_bridge.selection().await;

        match action.as_str() {
            "list" => {
                let provider = match parts.next() {
                    Some(provider) => ProviderId::from_str(provider)?,
                    None => selection.provider(),
                };
                let accounts = self.runtime_bridge.list_accounts(provider).await?;
                if accounts.is_empty() {
                    self.chat_widget.add_info_message(
                        format!("No {provider} accounts are registered."),
                        Some(format!("Add one with /account add {provider}.")),
                    );
                } else {
                    let rows = accounts
                        .into_iter()
                        .map(|account| {
                            let active = if account.active { " *" } else { "" };
                            format!(
                                "{}/{} - {}{}",
                                account.provider, account.id, account.label, active
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.chat_widget.add_info_message(
                        format!("{provider} accounts:\n{rows}"),
                        Some("* marks the native active account.".to_string()),
                    );
                }
            }
            "status" => {
                let provider = selection.provider();
                let active = self
                    .runtime_bridge
                    .list_accounts(provider)
                    .await?
                    .into_iter()
                    .find(|account| account.active);
                let account = active
                    .map(|account| format!("{}/{} ({})", provider, account.id, account.label))
                    .unwrap_or_else(|| "none".to_string());
                self.chat_widget.add_info_message(
                    format!(
                        "Runtime: {}\nModel: {}\nActive account: {account}",
                        provider,
                        selection.model.qualified()
                    ),
                    Some(ACCOUNT_USAGE.to_string()),
                );
            }
            "use" => {
                let target = parts
                    .next()
                    .ok_or_else(|| color_eyre::eyre::eyre!(ACCOUNT_USAGE))?;
                let (provider, account_id) = parse_account_target(target, selection.provider())?;
                let next = self.runtime_bridge.use_account(provider, account_id).await?;
                self.chat_widget.add_info_message(
                    format!(
                        "Active {} account changed; runtime generation invalidated ({})",
                        provider,
                        next.model.qualified()
                    ),
                    /*hint*/ None,
                );
            }
            "remove" => {
                let target = parts
                    .next()
                    .ok_or_else(|| color_eyre::eyre::eyre!(ACCOUNT_USAGE))?;
                let (provider, account_id) = parse_account_target(target, selection.provider())?;
                let generation = self
                    .runtime_bridge
                    .remove_account(provider, account_id.clone())
                    .await?;
                self.chat_widget.add_info_message(
                    format!(
                        "Removed {provider}/{account_id}; account generation is now {generation}"
                    ),
                    /*hint*/ None,
                );
            }
            "add" => {
                let provider = match parts.next() {
                    Some(provider) => ProviderId::from_str(provider)?,
                    None => selection.provider(),
                };
                match provider {
                    ProviderId::Cursor => {
                        let next = self.runtime_bridge.login_cursor(/*label_hint*/ None).await?;
                        self.chat_widget.add_info_message(
                            format!(
                                "Cursor login imported successfully; active runtime is {}",
                                next.model.qualified()
                            ),
                            /*hint*/ None,
                        );
                    }
                    ProviderId::OpenAi => {
                        if let Some(login_id) = self.pending_runtime_openai_login.take()
                            && let Err(error) =
                                self.runtime_bridge.cancel_openai_login(login_id).await
                        {
                            tracing::warn!(
                                error = %error,
                                "failed to cancel superseded runtime OpenAI login"
                            );
                        }
                        let login = self.runtime_bridge.start_openai_login().await?;
                        self.pending_runtime_openai_login = Some(login.login_id);
                        self.chat_widget.add_info_message(
                            format!(
                                "Continue OpenAI account login in your browser:\n{}",
                                login.auth_url
                            ),
                            Some(
                                "The account is imported only after Codex reports login completion."
                                    .to_string(),
                            ),
                        );
                    }
                }
            }
            _ => return Err(color_eyre::eyre::eyre!(ACCOUNT_USAGE)),
        }
        Ok(())
    }

    async fn runtime_quota_command(&mut self, args: &str) -> color_eyre::Result<()> {
        let selection = self.runtime_bridge.selection().await;
        let provider = if args.trim().is_empty() {
            selection.provider()
        } else {
            ProviderId::from_str(args.trim())?
        };
        let snapshot = self.runtime_bridge.quota_snapshot(provider).await?;
        if snapshot.accounts.is_empty() {
            self.chat_widget.add_info_message(
                format!("No quota data: {provider} has no registered accounts."),
                /*hint*/ None,
            );
            return Ok(());
        }

        let rows = snapshot
            .accounts
            .into_iter()
            .map(|account| {
                let windows = if account.quotas.is_empty() {
                    "no quota windows".to_string()
                } else {
                    account
                        .quotas
                        .iter()
                        .map(|quota| {
                            format!(
                                "{:?} {}/{} {:?}",
                                quota.window, quota.used, quota.limit, quota.status
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!(
                    "{}/{}: {} [{:?}]",
                    provider, account.account.id.0, windows, account.state
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.chat_widget.add_info_message(
            format!("{provider} quota:\n{rows}"),
            Some("Quota refresh uses the embedded subswap cache/backoff service.".to_string()),
        );
        Ok(())
    }

    async fn runtime_autoswap_command(&mut self, args: &str) -> color_eyre::Result<()> {
        if !args.trim().is_empty() && !args.trim().eq_ignore_ascii_case("run") {
            return Err(color_eyre::eyre::eyre!("Usage: /autoswap [run]"));
        }
        let decision = self.runtime_bridge.auto_swap_default().await?;
        let message = match decision {
            RuntimeAutoSwapDecision::Stay { provider, reason } => {
                format!("Autoswap stayed on {provider}: {reason}")
            }
            RuntimeAutoSwapDecision::Swap {
                provider,
                from,
                to,
                reason,
            } => format!(
                "Autoswap changed {provider} account {} -> {to}: {reason}",
                from.unwrap_or_else(|| "none".to_string())
            ),
            RuntimeAutoSwapDecision::Degraded { provider, reason } => {
                format!("Autoswap degraded for {provider}: {reason}")
            }
        };
        self.chat_widget.add_info_message(
            message,
            Some("Autoswap never crosses runtime providers.".to_string()),
        );
        Ok(())
    }
}

fn parse_account_target(
    value: &str,
    default_provider: ProviderId,
) -> color_eyre::Result<(ProviderId, String)> {
    if let Some((provider, account_id)) = value.split_once('/') {
        let provider = ProviderId::from_str(provider)?;
        if account_id.trim().is_empty() {
            return Err(color_eyre::eyre::eyre!(ACCOUNT_USAGE));
        }
        return Ok((provider, account_id.to_string()));
    }
    if value.trim().is_empty() {
        return Err(color_eyre::eyre::eyre!(ACCOUNT_USAGE));
    }
    Ok((default_provider, value.to_string()))
}
