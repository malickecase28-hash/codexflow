use crate::CursorAcpConfig;
use crate::ModelDescriptor;
use crate::ProviderCapabilities;
use crate::ProviderId;
use crate::RuntimeModelId;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum CursorModelDiscoveryError {
    #[error("no Cursor agent executable was found; install Cursor CLI or set CODEX_CURSOR_AGENT")]
    AgentUnavailable,
    #[error("Cursor model discovery failed using {executable}: {message}")]
    CommandFailed { executable: String, message: String },
    #[error("Cursor model discovery output was not UTF-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    #[error("Cursor returned no usable models")]
    EmptyCatalog,
}

/// Discover the model IDs available to the currently authenticated Cursor CLI.
///
/// Cursor's model inventory is account- and policy-dependent, so the harness
/// intentionally asks the installed agent every time the provider catalog is
/// refreshed instead of baking model names into CodexFlow. Cursor documents
/// `agent --list-models`/`agent models` as the supported non-interactive model
/// listing surface; the legacy `cursor-agent` binary remains a fallback.
pub async fn discover_cursor_models(
    config: &CursorAcpConfig,
) -> Result<Vec<ModelDescriptor>, CursorModelDiscoveryError> {
    let mut candidates = Vec::new();
    if let Some(executable) = config.executable.clone() {
        candidates.push(executable);
    } else if let Some(executable) = std::env::var_os("CODEX_CURSOR_AGENT") {
        candidates.push(PathBuf::from(executable));
    }
    if config.executable.is_none() {
        candidates.push(PathBuf::from("agent"));
        candidates.push(PathBuf::from("cursor-agent"));
    }

    let mut last_failure = None;
    for executable in candidates {
        let mut command = Command::new(&executable);
        command.arg("--list-models");
        if let Some(cwd) = &config.process_cwd {
            command.current_dir(cwd);
        }
        match command.output().await {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8(output.stdout)?;
                let models = parse_cursor_model_list(&stdout);
                if models.is_empty() {
                    last_failure = Some(CursorModelDiscoveryError::EmptyCatalog);
                    continue;
                }
                return Ok(models);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                last_failure = Some(CursorModelDiscoveryError::CommandFailed {
                    executable: executable.display().to_string(),
                    message: if stderr.is_empty() {
                        output.status.to_string()
                    } else {
                        stderr
                    },
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                last_failure = Some(CursorModelDiscoveryError::CommandFailed {
                    executable: executable.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    }

    Err(last_failure.unwrap_or(CursorModelDiscoveryError::AgentUnavailable))
}

/// Parse Cursor CLI's human-readable model listing conservatively.
///
/// The CLI has changed decoration over time (`*`, `>`, `(default)`, `(current)`),
/// while the first whitespace-delimited field has remained the selectable model
/// ID. Unknown headings are ignored and duplicate IDs are collapsed.
pub fn parse_cursor_model_list(output: &str) -> Vec<ModelDescriptor> {
    let mut models = BTreeMap::new();
    for raw_line in output.lines() {
        let mut line = raw_line.trim();
        while let Some(first) = line.chars().next() {
            if matches!(first, '*' | '>' | '-' | '•') {
                line = line[first.len_utf8()..].trim_start();
            } else {
                break;
            }
        }
        if line.is_empty() || line.ends_with(':') {
            continue;
        }

        let lower = line.to_ascii_lowercase();
        if lower.starts_with("available models")
            || lower.starts_with("model id")
            || lower.starts_with("models available")
        {
            continue;
        }

        let Some(id) = line.split_whitespace().next() else {
            continue;
        };
        if id.is_empty()
            || id.contains(':')
            || !id
                .chars()
                .any(|character| character.is_ascii_alphanumeric())
        {
            continue;
        }

        let remainder = line[id.len()..].trim();
        let display_name = strip_state_annotations(remainder);
        let display_name = if display_name.is_empty() {
            id.to_string()
        } else {
            display_name
        };
        let Ok(runtime_id) = RuntimeModelId::new(ProviderId::Cursor, id) else {
            continue;
        };
        models
            .entry(id.to_string())
            .or_insert_with(|| ModelDescriptor {
                id: runtime_id,
                display_name,
                capabilities: ProviderCapabilities::for_provider(ProviderId::Cursor),
                parameters: Vec::new(),
                metadata: json!({ "source": "cursor-cli", "raw": raw_line.trim() }),
            });
    }
    models.into_values().collect()
}

fn strip_state_annotations(value: &str) -> String {
    value
        .replace("(default)", "")
        .replace("(current)", "")
        .replace("[default]", "")
        .replace("[current]", "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decorated_cursor_model_inventory_without_hard_coding_models() {
        let models = parse_cursor_model_list(
            "Available models:\n> gpt-x GPT X (current)\n* composer-y Composer Y (default)\n- gemini-z\n",
        );
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id.qualified(), "cursor/composer-y");
        assert_eq!(models[0].display_name, "Composer Y");
        assert_eq!(models[1].id.qualified(), "cursor/gemini-z");
        assert_eq!(models[2].id.qualified(), "cursor/gpt-x");
        assert_eq!(models[2].display_name, "GPT X");
    }

    #[test]
    fn duplicate_model_ids_are_collapsed() {
        let models = parse_cursor_model_list("gpt-x GPT X\ngpt-x GPT X (current)\n");
        assert_eq!(models.len(), 1);
    }
}
