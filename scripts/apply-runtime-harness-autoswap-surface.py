from pathlib import Path

path = Path("codex-rs/tui/src/app/runtime_commands.rs")
text = path.read_text()
old = '''    async fn runtime_autoswap_command(&mut self, args: &str) -> color_eyre::Result<()> {
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
'''
new = '''    async fn runtime_autoswap_command(&mut self, args: &str) -> color_eyre::Result<()> {
        let action = args.trim().to_ascii_lowercase();
        match action.as_str() {
            "" | "status" => {
                let status = self.runtime_bridge.auto_swap_status();
                self.chat_widget.add_info_message(
                    format!(
                        "Autoswap: {} (threshold {:.0}%)",
                        if status.enabled { "on" } else { "off" },
                        status.threshold * 100.0
                    ),
                    Some("Use /autoswap on|off|run. Autoswap never crosses providers.".to_string()),
                );
            }
            "on" | "off" => {
                let enabled = action == "on";
                let status = self.runtime_bridge.set_auto_swap_enabled(enabled)?;
                self.chat_widget.add_info_message(
                    format!(
                        "Autoswap {} at {:.0}% threshold.",
                        if status.enabled { "enabled" } else { "disabled" },
                        status.threshold * 100.0
                    ),
                    Some("The setting is persisted in embedded subswap configuration.".to_string()),
                );
            }
            "run" => {
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
            }
            _ => {
                return Err(color_eyre::eyre::eyre!(
                    "Usage: /autoswap [status|on|off|run]"
                ));
            }
        }
        Ok(())
    }
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f"expected one autoswap command implementation, found {count}")
path.write_text(text.replace(old, new, 1))
