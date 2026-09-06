from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one patch anchor, found {count}")
    target.write_text(text.replace(old, new, 1))


# Runtime commands execute at App scope. Opening the model popup is crate-visible,
# but the queue-suppression helper is intentionally private to chatwidget.
replace_once(
    "codex-rs/tui/src/app/runtime_commands.rs",
    "                    self.chat_widget.open_model_popup();\n"
    "                    self.chat_widget.defer_input_until_settings_applied();\n",
    "                    self.chat_widget.open_model_popup();\n",
)

replace_once(
    "codex-rs/tui/src/app.rs",
    "mod recap;\nmod replay_filter;\n",
    "mod recap;\nmod replay_filter;\nmod runtime_commands;\n",
)

replace_once(
    "codex-rs/tui/src/app_event.rs",
    "    /// Update the current model slug in the running app and widget.\n"
    "    UpdateModel(String),\n",
    "    /// Route a multi-runtime slash command to the process-owned runtime bridge.\n"
    "    RuntimeSlashCommand { command: String, args: String },\n\n"
    "    /// Update the current model slug in the running app and widget.\n"
    "    UpdateModel(String),\n",
)

replace_once(
    "codex-rs/tui/src/app/event_dispatch.rs",
    "            AppEvent::UpdateModel(model) => {\n",
    "            AppEvent::RuntimeSlashCommand { command, args } => {\n"
    "                self.handle_runtime_slash_command(command, args).await;\n"
    "            }\n"
    "            AppEvent::UpdateModel(model) => {\n",
)

replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "    Model,\n    Ide,\n",
    "    Model,\n    Provider,\n    Account,\n    Quota,\n    Autoswap,\n    Ide,\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    '            SlashCommand::Model => "choose what model and reasoning effort to use",\n',
    '            SlashCommand::Model => "choose a model for the active runtime provider",\n'
    '            SlashCommand::Provider => "show or switch the active runtime provider",\n'
    '            SlashCommand::Account => "list, add, select, remove, or inspect runtime accounts",\n'
    '            SlashCommand::Quota => "refresh quota for a runtime provider",\n'
    '            SlashCommand::Autoswap => "run same-provider account autoswap policy",\n',
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "            SlashCommand::Review\n",
    "            SlashCommand::Model\n"
    "                | SlashCommand::Provider\n"
    "                | SlashCommand::Account\n"
    "                | SlashCommand::Quota\n"
    "                | SlashCommand::Autoswap\n"
    "                | SlashCommand::Review\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "            SlashCommand::New\n",
    "            SlashCommand::Provider\n"
    "            | SlashCommand::Account\n"
    "            | SlashCommand::Autoswap\n"
    "            | SlashCommand::New\n",
)
replace_once(
    "codex-rs/tui/src/slash_command.rs",
    "            | SlashCommand::Usage\n            | SlashCommand::DebugConfig\n",
    "            | SlashCommand::Usage\n"
    "            | SlashCommand::Quota\n"
    "            | SlashCommand::DebugConfig\n",
)

old_model_dispatch = '''            SlashCommand::Model => {
                self.open_model_popup();
                self.defer_input_until_settings_applied();
            }
'''
new_model_dispatch = '''            SlashCommand::Model => {
                self.app_event_tx.send(AppEvent::RuntimeSlashCommand {
                    command: "model".to_string(),
                    args: String::new(),
                });
            }
            SlashCommand::Provider => {
                self.app_event_tx.send(AppEvent::RuntimeSlashCommand {
                    command: "provider".to_string(),
                    args: String::new(),
                });
            }
            SlashCommand::Account => {
                self.app_event_tx.send(AppEvent::RuntimeSlashCommand {
                    command: "account".to_string(),
                    args: String::new(),
                });
            }
            SlashCommand::Quota => {
                self.app_event_tx.send(AppEvent::RuntimeSlashCommand {
                    command: "quota".to_string(),
                    args: String::new(),
                });
            }
            SlashCommand::Autoswap => {
                self.app_event_tx.send(AppEvent::RuntimeSlashCommand {
                    command: "autoswap".to_string(),
                    args: String::new(),
                });
            }
'''
replace_once(
    "codex-rs/tui/src/chatwidget/slash_dispatch.rs",
    old_model_dispatch,
    new_model_dispatch,
)
replace_once(
    "codex-rs/tui/src/chatwidget/slash_dispatch.rs",
    "        match cmd {\n            SlashCommand::Export if trimmed.is_empty() => self.show_transcript_export_popup(),\n",
    "        match cmd {\n"
    "            SlashCommand::Model\n"
    "            | SlashCommand::Provider\n"
    "            | SlashCommand::Account\n"
    "            | SlashCommand::Quota\n"
    "            | SlashCommand::Autoswap => {\n"
    "                self.app_event_tx.send(AppEvent::RuntimeSlashCommand {\n"
    "                    command: cmd.command().to_string(),\n"
    "                    args: trimmed.to_string(),\n"
    "                });\n"
    "            }\n"
    "            SlashCommand::Export if trimmed.is_empty() => self.show_transcript_export_popup(),\n",
)
